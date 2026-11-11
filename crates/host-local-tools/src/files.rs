use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use minicore_host_workspace::{EditOutcome, FileSnapshot, Workspace};
use minicore_runtime::{
    ToolCallId,
    tools::{ToolContext, ToolError, ToolExecutionOutcome},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Binding, CapturedEdit, check, completed};

#[cfg(windows)]
mod windows;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadInput {
    path: String,
    #[serde(default = "first")]
    offset: usize,
    limit: usize,
}
fn first() -> usize {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditInput {
    path: String,
    old_text: String,
    new_text: String,
    #[serde(default)]
    replace_all: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WriteInput {
    path: String,
    content: String,
}

enum Failure {
    Io(io::Error),
    Invalid(&'static str),
    Conflict(&'static str),
    Control(ToolError),
}
impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<ToolError> for Failure {
    fn from(error: ToolError) -> Self {
        Self::Control(error)
    }
}
impl Failure {
    fn value(&self) -> Value {
        match self {
            Self::Io(error) => json!({"code":"filesystem","message":error.to_string()}),
            Self::Invalid(message) => json!({"code":"invalid_path_or_content","message":message}),
            Self::Conflict(message) => json!({"code":"conflict","message":message}),
            Self::Control(error) => json!({"code":"stopped","message":error.to_string()}),
        }
    }
    fn result(self) -> Result<ToolExecutionOutcome, ToolError> {
        match self {
            Self::Control(error) => Err(error),
            other => completed(json!({"status":"failed","error":other.value()})),
        }
    }
}

pub(crate) fn normalize(path: &str, root: bool) -> Result<PathBuf, &'static str> {
    if path.contains('\0') {
        return Err("path contains NUL");
    }
    let mut relative = PathBuf::new();
    for part in Path::new(path).components() {
        match part {
            Component::Normal(name) => {
                #[cfg(windows)]
                if name
                    .to_str()
                    .is_none_or(|name| name.contains(':') || name.ends_with(['.', ' ']))
                {
                    return Err("alternate streams and ambiguous Windows paths are not supported");
                }
                relative.push(name);
            }
            Component::CurDir => (),
            _ => return Err("path must be relative and cannot contain '..'"),
        }
    }
    if relative.as_os_str().is_empty() && !root {
        return Err("file path is empty");
    }
    Ok(relative)
}

pub(crate) fn directory(workspace: &Workspace, path: &str) -> io::Result<PathBuf> {
    let relative = normalize(path, true).map_err(io::Error::other)?;
    let target = fs::canonicalize(workspace.root().join(relative))?;
    if !target.starts_with(workspace.root()) || !target.is_dir() {
        return Err(io::Error::other(
            "cwd must be a directory inside the bound workspace",
        ));
    }
    Ok(target)
}

pub(crate) async fn read(
    binding: Arc<Binding>,
    input: ReadInput,
    context: ToolContext,
) -> Result<ToolExecutionOutcome, ToolError> {
    if input.offset == 0 || input.limit == 0 {
        return Err(ToolError::InvalidInvocation);
    }
    let result =
        tokio::task::spawn_blocking(move || read_sync(&binding.workspace, input, &context))
            .await
            .map_err(|_| ToolError::Internal)?;
    match result {
        Ok(value) => completed(value),
        Err(error) => error.result(),
    }
}

fn read_sync(
    workspace: &Workspace,
    input: ReadInput,
    context: &ToolContext,
) -> Result<Value, Failure> {
    check(context)?;
    let relative = normalize(&input.path, true).map_err(Failure::Invalid)?;
    let target = fs::canonicalize(workspace.root().join(&relative))?;
    if !target.starts_with(workspace.root()) {
        return Err(Failure::Invalid("path leaves the workspace"));
    }
    let metadata = fs::metadata(&target)?;
    if metadata.is_dir() {
        let mut entries = Vec::new();
        for entry in fs::read_dir(target)? {
            check(context)?;
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Failure::Invalid("directory contains a non-UTF-8 name"))?;
            let kind = entry.file_type()?;
            entries.push((
                name,
                if kind.is_dir() {
                    "directory"
                } else if kind.is_file() {
                    "file"
                } else if kind.is_symlink() {
                    "symlink"
                } else {
                    "other"
                },
            ));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let total = entries.len();
        let selected: Vec<_> = entries
            .into_iter()
            .skip(input.offset - 1)
            .take(input.limit)
            .map(|(name, kind)| json!({"name":name,"kind":kind}))
            .collect();
        let next = (input.offset - 1).saturating_add(selected.len());
        return Ok(
            json!({"status":"ok","kind":"directory","path":input.path,"offset":input.offset,
            "entries":selected,"next_offset":(next < total).then_some(next + 1),"total_entries":total}),
        );
    }
    if !metadata.is_file() {
        return Err(Failure::Invalid("path is not a regular file or directory"));
    }
    let file = File::open(target)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(Failure::Invalid("path is not a regular file"));
    }
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut text = String::new();
    let mut number = 1usize;
    let mut returned = 0usize;
    let mut eof = false;
    loop {
        check(context)?;
        if returned == input.limit {
            break;
        }
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            eof = true;
            break;
        }
        if number >= input.offset {
            let content = std::str::from_utf8(&line)
                .map_err(|_| Failure::Invalid("requested content is not UTF-8"))?;
            if content.contains('\0') {
                return Err(Failure::Invalid("requested content is binary"));
            }
            text.push_str(content);
            returned += 1;
        }
        number = number
            .checked_add(1)
            .ok_or(Failure::Invalid("line number overflow"))?;
    }
    if !eof {
        eof = reader.fill_buf()?.is_empty();
    }
    let after = reader.get_ref().metadata()?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(Failure::Conflict("file changed during read"));
    }
    Ok(
        json!({"status":"ok","kind":"file","path":input.path,"offset":input.offset,
        "lines":returned,"text":text,"next_offset":(!eof).then_some(number),"eof":eof}),
    )
}

pub(crate) async fn edit(
    binding: Arc<Binding>,
    id: ToolCallId,
    input: EditInput,
    context: ToolContext,
) -> Result<ToolExecutionOutcome, ToolError> {
    if input.old_text.is_empty() || input.old_text.contains('\0') || input.new_text.contains('\0') {
        return Err(ToolError::InvalidInvocation);
    }
    mutate(binding, id, Mutation::Edit(input), context).await
}
pub(crate) async fn write(
    binding: Arc<Binding>,
    id: ToolCallId,
    input: WriteInput,
    context: ToolContext,
) -> Result<ToolExecutionOutcome, ToolError> {
    if input.content.contains('\0') {
        return Err(ToolError::InvalidInvocation);
    }
    mutate(binding, id, Mutation::Write(input), context).await
}
enum Mutation {
    Edit(EditInput),
    Write(WriteInput),
}
impl Mutation {
    fn path(&self) -> &str {
        match self {
            Self::Edit(input) => &input.path,
            Self::Write(input) => &input.path,
        }
    }
}

async fn mutate(
    binding: Arc<Binding>,
    id: ToolCallId,
    input: Mutation,
    context: ToolContext,
) -> Result<ToolExecutionOutcome, ToolError> {
    let guard = tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => return Err(ToolError::Cancelled),
        _ = tokio::time::sleep_until(context.deadline.into()) => return Err(ToolError::TimedOut),
        guard = binding.workspace.begin_mutation() => guard,
    };
    check(&context)?;
    tokio::task::spawn_blocking(move || {
        // The file worker owns the whole source/commit/observation boundary,
        // even if its async waiter is dropped. Other Workspace clones wait.
        let _guard = guard;
        mutate_sync(&binding, &id, input, &context)
    })
    .await
    .map_err(|_| ToolError::Internal)?
}

fn write_target(workspace: &Workspace, path: &str) -> Result<(String, PathBuf), Failure> {
    let relative = normalize(path, false).map_err(Failure::Invalid)?;
    let mut target = workspace.root().to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            unreachable!()
        };
        #[cfg(not(any(windows, target_os = "macos")))]
        let git_metadata = name == ".git";
        #[cfg(any(windows, target_os = "macos"))]
        let git_metadata = name
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(".git"));
        if git_metadata {
            return Err(Failure::Invalid("writes to Git metadata are not supported"));
        }
        target.push(name);
        match fs::symlink_metadata(&target) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(Failure::Invalid(
                    "writes through symlinks are not supported",
                ));
            }
            Ok(metadata) if target != workspace.root().join(&relative) && !metadata.is_dir() => {
                return Err(Failure::Invalid("parent is not a directory"));
            }
            Ok(metadata) if target == workspace.root().join(&relative) && !metadata.is_file() => {
                return Err(Failure::Invalid("target is not a regular file"));
            }
            Ok(_) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    let path = relative
        .to_str()
        .ok_or(Failure::Invalid("non-UTF-8 path"))?;
    #[cfg(windows)]
    let path = path.replace('\\', "/");
    Ok((path.to_owned(), target))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.is_symlink()
}

fn capture(path: &Path, context: Option<&ToolContext>) -> Result<FileSnapshot, Failure> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(FileSnapshot::Missing),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Err(Failure::Invalid("target is not a regular file"));
    }
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(Failure::Invalid("target is not a regular file"));
    }
    let mut bytes = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        if let Some(context) = context {
            check(context)?;
        }
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    let after = file.metadata()?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(Failure::Conflict("file changed during capture"));
    }
    Ok(FileSnapshot::Content(bytes))
}

fn mutate_sync(
    binding: &Binding,
    id: &ToolCallId,
    input: Mutation,
    context: &ToolContext,
) -> Result<ToolExecutionOutcome, ToolError> {
    check(context)?;
    let (path, target) = match write_target(&binding.workspace, input.path()) {
        Ok(value) => value,
        Err(error) => return error.result(),
    };
    let before = match capture(&target, Some(context)) {
        Ok(value) => value,
        Err(error) => return error.result(),
    };
    let mut applied = false;
    let result = (|| -> Result<Value, Failure> {
        let (bytes, replacements) = match input {
            Mutation::Write(input) => (input.content.into_bytes(), None),
            Mutation::Edit(input) => {
                let FileSnapshot::Content(bytes) = &before else {
                    return Err(Failure::Invalid("edit target does not exist"));
                };
                let source = std::str::from_utf8(bytes)
                    .map_err(|_| Failure::Invalid("edit target is not UTF-8"))?;
                if source.contains('\0') {
                    return Err(Failure::Invalid("edit target is binary"));
                }
                let count = source.match_indices(&input.old_text).count();
                if count == 0 || (count > 1 && !input.replace_all) {
                    return Err(Failure::Conflict(
                        "edit requires exactly one match, or replace_all for multiple matches",
                    ));
                }
                let bytes = source
                    .replace(&input.old_text, &input.new_text)
                    .into_bytes();
                (bytes, Some(count))
            }
        };
        check(context)?;
        let parent = target.parent().expect("validated file has a parent");
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        if matches!(before, FileSnapshot::Content(_)) {
            #[cfg(not(windows))]
            temporary
                .as_file()
                .set_permissions(fs::metadata(&target)?.permissions())?;
            #[cfg(windows)]
            windows::preserve_permissions(&target, &temporary)?;
        }
        for chunk in bytes.chunks(64 * 1024) {
            check(context)?;
            temporary.write_all(chunk)?;
        }
        temporary.as_file().sync_all()?;
        check(context)?;
        // Detect a changed target before committing. No cross-process lock or
        // hostile concurrent filesystem mutation is claimed by this local tool.
        write_target(&binding.workspace, &path)?;
        if capture(&target, Some(context))? != before {
            return Err(Failure::Conflict(
                "target changed before atomic replacement",
            ));
        }
        check(context)?;
        temporary
            .persist(&target)
            .map_err(|error| Failure::Io(error.error))?;
        applied = true;
        #[cfg(not(windows))]
        File::open(parent)?.sync_all()?;
        #[cfg(windows)]
        windows::sync_directory(parent)?;
        Ok(json!({"status":"applied","path":path,"bytes":bytes.len(),"replacements":replacements}))
    })();
    // This capture is cleanup after an attempted mutation. Cancellation cannot
    // erase or pretend to roll back an already committed file replacement.
    let after = capture(&target, None).unwrap_or(FileSnapshot::Unknown);
    binding
        .sink
        .edit(
            id,
            CapturedEdit {
                path,
                before,
                after,
                outcome: if applied {
                    EditOutcome::Applied
                } else {
                    EditOutcome::NotApplied
                },
            },
        )
        .map_err(|_| ToolError::Internal)?;
    match result {
        Ok(value) => completed(value),
        Err(Failure::Control(error)) => Err(error),
        Err(error) => completed(
            json!({"status":"failed","write_outcome":if applied { "applied" } else { "not_applied" },"error":error.value()}),
        ),
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn unsupported_command() -> Result<ToolExecutionOutcome, ToolError> {
    crate::business_error(
        "unsupported_platform",
        "bash execution is supported only on Unix and Windows",
    )
}
