use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

use crate::{
    QueryContext, Result, Workspace, WorkspaceError,
    context::blocking,
    workspace::{normalize, relative_path_text},
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialOrd, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusEntryKind {
    Ordinary,
    Renamed,
    Unmerged,
    Untracked,
}

#[derive(Clone, Debug, Eq, Ord, PartialOrd, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusEntry {
    pub path: String,
    pub kind: StatusEntryKind,
    pub index_status: Option<String>,
    pub worktree_status: Option<String>,
    pub original_path: Option<String>,
    pub submodule: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatusResult {
    pub repo_available: bool,
    pub head_oid: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub entries: Vec<StatusEntry>,
    pub complete: bool,
}

impl Workspace {
    /// A non-repository directory is a real `repo_available: false` result.
    /// Missing Git, unreadable/corrupt repositories and missing roots are errors.
    pub async fn status(&self, context: QueryContext) -> Result<StatusResult> {
        let Some(repository) = Repository::discover(self, &context).await? else {
            return Ok(StatusResult {
                repo_available: false,
                head_oid: None,
                branch: None,
                detached: false,
                entries: Vec::new(),
                complete: true,
            });
        };
        repository.status(&context).await
    }
}

#[derive(Clone)]
pub(crate) struct Repository {
    pub(crate) workspace: Workspace,
    root: PathBuf,
    prefix: PathBuf,
}

impl Repository {
    pub(crate) async fn discover(
        workspace: &Workspace,
        context: &QueryContext,
    ) -> Result<Option<Self>> {
        let binding = workspace.clone();
        blocking(context.clone(), move |_| binding.directory("")).await?;
        let result = git(
            workspace.root(),
            &["rev-parse", "--is-inside-work-tree"],
            context,
        )
        .await?;
        if !result.status.success() {
            if result
                .stderr
                .starts_with(b"fatal: not a git repository (or any")
            {
                let root = workspace.root().to_owned();
                let marker_exists = blocking(context.clone(), move |context| {
                    for directory in root.ancestors() {
                        context.check()?;
                        match fs::symlink_metadata(directory.join(".git")) {
                            Ok(metadata) => {
                                // Empty ancestor placeholders carry no repository
                                // state. An initialized but damaged directory does.
                                if !metadata.is_dir()
                                    || fs::read_dir(directory.join(".git"))?
                                        .next()
                                        .transpose()?
                                        .is_some()
                                {
                                    return Ok(true);
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                            Err(error) => return Err(error.into()),
                        }
                    }
                    Ok(false)
                })
                .await?;
                if !marker_exists {
                    return Ok(None);
                }
            }
            return Err(result.failure("repository discovery"));
        }
        if result.stdout == b"false\n" {
            return Ok(None);
        }
        if result.stdout != b"true\n" {
            return Err(WorkspaceError::GitOutput("repository discovery"));
        }
        let result = git(workspace.root(), &["rev-parse", "--show-toplevel"], context)
            .await?
            .success("repository root")?;
        let text = std::str::from_utf8(result.stdout.strip_suffix(b"\n").unwrap_or(&result.stdout))
            .map_err(|_| WorkspaceError::NonUtf8Path)?;
        let root = fs::canonicalize(PathBuf::from(text))?;
        let prefix = workspace
            .root()
            .strip_prefix(&root)
            .map_err(|_| WorkspaceError::OutsideWorkspace)?
            .to_owned();
        Ok(Some(Self {
            workspace: workspace.clone(),
            root,
            prefix,
        }))
    }

    pub(crate) async fn status(&self, context: &QueryContext) -> Result<StatusResult> {
        self.check_content_filters(context).await?;
        // Porcelain v2 always uses repository-relative paths, including when
        // the bound workspace is a subdirectory. The literal pathspec limits
        // the observation to that workspace.
        let result = git(
            self.workspace.root(),
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
                "--renames",
                "--ignore-submodules=none",
                "--",
                ".",
            ],
            context,
        )
        .await?
        .success("status")?;
        let mut status = StatusResult {
            repo_available: true,
            head_oid: None,
            branch: None,
            detached: false,
            entries: Vec::new(),
            complete: true,
        };
        let mut records = result
            .stdout
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty());
        while let Some(record) = records.next() {
            context.check()?;
            let record = std::str::from_utf8(record).map_err(|_| WorkspaceError::NonUtf8Path)?;
            if let Some(value) = record.strip_prefix("# branch.oid ") {
                if value != "(initial)" {
                    status.head_oid = Some(value.to_owned());
                }
            } else if let Some(value) = record.strip_prefix("# branch.head ") {
                status.detached = value == "(detached)";
                if !status.detached {
                    status.branch = Some(value.to_owned());
                }
            } else if record.starts_with('#') {
                continue;
            } else if let Some(path) = record.strip_prefix("? ") {
                status.entries.push(StatusEntry {
                    path: self.relative(path)?,
                    kind: StatusEntryKind::Untracked,
                    submodule: false,
                    index_status: None,
                    worktree_status: None,
                    original_path: None,
                });
            } else {
                let (count, kind) = match record.as_bytes().first() {
                    Some(b'1') => (9, StatusEntryKind::Ordinary),
                    Some(b'2') => (10, StatusEntryKind::Renamed),
                    Some(b'u') => (11, StatusEntryKind::Unmerged),
                    _ => return Err(WorkspaceError::GitOutput("status")),
                };
                let fields: Vec<_> = record.splitn(count, ' ').collect();
                if fields.len() != count || fields[1].len() != 2 || !fields[1].is_ascii() {
                    return Err(WorkspaceError::GitOutput("status"));
                }
                let xy = fields[1].as_bytes();
                let original_path = if kind == StatusEntryKind::Renamed {
                    let original = records.next().ok_or(WorkspaceError::GitOutput("rename"))?;
                    Some(self.relative(
                        std::str::from_utf8(original).map_err(|_| WorkspaceError::NonUtf8Path)?,
                    )?)
                } else {
                    None
                };
                status.entries.push(StatusEntry {
                    path: self.relative(fields[count - 1])?,
                    kind,
                    original_path,
                    submodule: fields[2].starts_with('S'),
                    index_status: (xy[0] != b'.').then(|| (xy[0] as char).to_string()),
                    worktree_status: (xy[1] != b'.').then(|| (xy[1] as char).to_string()),
                });
            }
        }
        status.entries.sort();
        Ok(status)
    }

    /// Status can run clean/process filters, including in initialized
    /// submodules. Refuse those actual bindings before starting status rather
    /// than disabling the filters and reporting different Git semantics.
    async fn check_content_filters(&self, context: &QueryContext) -> Result<()> {
        let mut pending = vec![self.clone()];
        while let Some(repository) = pending.pop() {
            context.check()?;
            let config = git(
                &repository.root,
                &[
                    "config",
                    "--null",
                    "--get-regexp",
                    r"^filter\..*\.(clean|process)$",
                ],
                context,
            )
            .await?;
            if !config.status.success() && config.status.code() != Some(1) {
                return Err(config.failure("filter configuration"));
            }
            // Git configuration uses the last value of each driver setting.
            let mut settings = BTreeMap::new();
            for record in config
                .stdout
                .split(|byte| *byte == 0)
                .filter(|record| !record.is_empty())
            {
                let (key, value) = split_record(record, b'\n', "filter configuration")?;
                let key = std::str::from_utf8(key)
                    .map_err(|_| WorkspaceError::GitOutput("filter configuration"))?;
                settings.insert(key.to_owned(), !value.is_empty());
            }
            let drivers: BTreeSet<_> = settings
                .iter()
                .filter(|(_, active)| **active)
                .filter_map(|(key, _)| {
                    key.strip_prefix("filter.").and_then(|key| {
                        key.strip_suffix(".clean")
                            .or_else(|| key.strip_suffix(".process"))
                    })
                })
                .collect();
            let tracked = git(
                repository.workspace.root(),
                &["ls-files", "--stage", "--full-name", "-z", "--", "."],
                context,
            )
            .await?
            .success("tracked paths")?;
            let mut files = BTreeSet::new();
            let mut submodules = BTreeSet::new();
            for record in tracked
                .stdout
                .split(|byte| *byte == 0)
                .filter(|record| !record.is_empty())
            {
                context.check()?;
                let (header, path) = split_record(record, b'\t', "tracked paths")?;
                let path = std::str::from_utf8(path).map_err(|_| WorkspaceError::NonUtf8Path)?;
                normalize(path, false)?;
                let mode = header
                    .split(|byte| *byte == b' ')
                    .next()
                    .ok_or(WorkspaceError::GitOutput("tracked paths"))?;
                if mode == b"160000" {
                    submodules.insert(path);
                } else if mode == b"100644" || mode == b"100755" {
                    files.insert(path);
                }
            }
            if !drivers.is_empty() && !files.is_empty() {
                let mut input = Vec::new();
                for path in files {
                    input.extend_from_slice(path.as_bytes());
                    input.push(0);
                }
                let attributes = git_with_input(
                    &repository.root,
                    &["check-attr", "-z", "--stdin", "filter"],
                    Some(input),
                    context,
                )
                .await?
                .success("filter attributes")?;
                let mut fields = attributes.stdout.split(|byte| *byte == 0);
                while let Some(path) = fields.next().filter(|path| !path.is_empty()) {
                    context.check()?;
                    let attribute = fields
                        .next()
                        .ok_or(WorkspaceError::GitOutput("filter attributes"))?;
                    let value = fields
                        .next()
                        .ok_or(WorkspaceError::GitOutput("filter attributes"))?;
                    if attribute != b"filter" || !files_path_is_relative(path) {
                        return Err(WorkspaceError::GitOutput("filter attributes"));
                    }
                    let value = std::str::from_utf8(value)
                        .map_err(|_| WorkspaceError::GitOutput("filter attributes"))?;
                    if drivers.contains(value) {
                        return Err(WorkspaceError::UnsupportedFilter);
                    }
                }
            }
            for path in submodules {
                let directory = repository.root.join(path);
                let boundary = self.workspace.root().to_owned();
                let directory = blocking(context.clone(), move |_| {
                    let metadata = match fs::symlink_metadata(&directory) {
                        Ok(metadata) => metadata,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            return Ok(None);
                        }
                        Err(error) => return Err(error.into()),
                    };
                    if metadata.is_symlink() {
                        return Err(WorkspaceError::UnsupportedChange);
                    }
                    if !metadata.is_dir() {
                        return Ok(None);
                    }
                    let directory = fs::canonicalize(directory)?;
                    if !directory.starts_with(&boundary) {
                        return Err(WorkspaceError::OutsideWorkspace);
                    }
                    match fs::symlink_metadata(directory.join(".git")) {
                        Ok(_) => Ok(Some(directory)),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                        Err(error) => Err(error.into()),
                    }
                })
                .await?;
                if let Some(directory) = directory {
                    let workspace = Workspace::open(&directory).await?;
                    let child = Repository::discover(&workspace, context)
                        .await?
                        .ok_or(WorkspaceError::UnsupportedChange)?;
                    if child.root != directory {
                        return Err(WorkspaceError::UnsupportedChange);
                    }
                    pending.push(child);
                }
            }
        }
        Ok(())
    }

    fn relative(&self, path: &str) -> Result<String> {
        let path = normalize(path, false)?;
        let relative = path
            .strip_prefix(&self.prefix)
            .map_err(|_| WorkspaceError::OutsideWorkspace)?;
        relative_path_text(relative)
    }

    fn repo_path(&self, path: &str) -> Result<String> {
        relative_path_text(&self.prefix.join(normalize(path, false)?))
    }

    pub(crate) async fn head_bytes(
        &self,
        oid: Option<&str>,
        path: &str,
        context: &QueryContext,
    ) -> Result<crate::FileSnapshot> {
        let Some(oid) = oid else {
            return Ok(crate::FileSnapshot::Missing);
        };
        let path = self.repo_path(path)?;
        let result = git(&self.root, &["ls-tree", "-z", oid, "--", &path], context)
            .await?
            .success("HEAD entry")?;
        self.blob_from_entry(&result.stdout, false, context).await
    }

    pub(crate) async fn index_bytes(
        &self,
        path: &str,
        context: &QueryContext,
    ) -> Result<crate::FileSnapshot> {
        let path = self.repo_path(path)?;
        let result = git(
            &self.root,
            &["ls-files", "--stage", "-z", "--", &path],
            context,
        )
        .await?
        .success("index entry")?;
        self.blob_from_entry(&result.stdout, true, context).await
    }

    async fn blob_from_entry(
        &self,
        bytes: &[u8],
        index: bool,
        context: &QueryContext,
    ) -> Result<crate::FileSnapshot> {
        if bytes.is_empty() {
            return Ok(crate::FileSnapshot::Missing);
        }
        let records: Vec<_> = bytes
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .collect();
        if records.len() != 1 {
            return Err(WorkspaceError::UnsupportedChange);
        }
        let header = records[0]
            .split(|byte| *byte == b'\t')
            .next()
            .ok_or(WorkspaceError::GitOutput("blob entry"))?;
        let header =
            std::str::from_utf8(header).map_err(|_| WorkspaceError::GitOutput("blob entry"))?;
        let fields: Vec<_> = header.split(' ').collect();
        if fields.len() != 3 {
            return Err(WorkspaceError::GitOutput("blob entry"));
        }
        if !matches!(fields[0], "100644" | "100755" | "120000")
            || (index && fields[2] != "0")
            || (!index && fields[1] != "blob")
        {
            return Err(WorkspaceError::UnsupportedChange);
        }
        let oid = fields[if index { 1 } else { 2 }];
        let result = git(&self.root, &["cat-file", "blob", oid], context)
            .await?
            .success("blob read")?;
        Ok(crate::FileSnapshot::Content(result.stdout))
    }
}

fn split_record<'a>(
    record: &'a [u8],
    separator: u8,
    operation: &'static str,
) -> Result<(&'a [u8], &'a [u8])> {
    let index = record
        .iter()
        .position(|byte| *byte == separator)
        .ok_or(WorkspaceError::GitOutput(operation))?;
    Ok((&record[..index], &record[index + 1..]))
}

fn files_path_is_relative(path: &[u8]) -> bool {
    std::str::from_utf8(path).is_ok_and(|path| normalize(path, false).is_ok())
}

struct GitOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl GitOutput {
    fn failure(&self, operation: &'static str) -> WorkspaceError {
        WorkspaceError::GitFailed {
            operation,
            code: self.status.code(),
        }
    }
    fn success(self, operation: &'static str) -> Result<Self> {
        if self.status.success() {
            Ok(self)
        } else {
            Err(self.failure(operation))
        }
    }
}

async fn git(root: &Path, args: &[&str], context: &QueryContext) -> Result<GitOutput> {
    git_with_input(root, args, None, context).await
}

async fn git_with_input(
    root: &Path,
    args: &[&str],
    input: Option<Vec<u8>>,
    context: &QueryContext,
) -> Result<GitOutput> {
    context.check()?;
    let root = root.to_owned();
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let context = context.clone();
    // This process owner outlives an abandoned caller. Input writing, output
    // draining and child reaping all stay inside it, including cancellation.
    tokio::spawn(async move {
        context.check()?;
        let mut command = Command::new("git");
        command.args(["--no-pager", "-c", "core.fsmonitor=false"])
            .args(args).current_dir(root).stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true)
            .env("LC_ALL", "C").env("GIT_OPTIONAL_LOCKS", "0").env("GIT_LITERAL_PATHSPECS", "1")
            .env("GIT_NO_LAZY_FETCH", "1").env("GIT_TERMINAL_PROMPT", "0");
        // GIT_CONFIG redirects `git config` alone, so it would make the filter
        // preflight inspect different configuration than the later status.
        for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM", "GIT_CONFIG"] { command.env_remove(name); }
        let mut child = command.spawn().map_err(WorkspaceError::GitUnavailable)?;
        let writer = input.map(|bytes| {
            let mut stdin = child.stdin.take().expect("piped stdin");
            tokio::spawn(async move { stdin.write_all(&bytes).await?; stdin.shutdown().await })
        });
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let out = tokio::spawn(async move { let mut bytes = Vec::new(); stdout.read_to_end(&mut bytes).await.map(|_| bytes) });
        let err = tokio::spawn(async move { let mut bytes = Vec::new(); stderr.read_to_end(&mut bytes).await.map(|_| bytes) });
        let status = tokio::select! {
            biased;
            reason = context.stopped() => { child.kill().await.map_err(WorkspaceError::Io).and(Err(reason)) },
            status = child.wait() => status.map_err(WorkspaceError::Io),
        };
        // Join all pipe tasks before propagating a cancellation or I/O error.
        let written = match writer { Some(writer) => writer.await.map_err(|_| WorkspaceError::WorkerFailed).and_then(|written| written.map_err(WorkspaceError::Io)), None => Ok(()) };
        let stdout = out.await;
        let stderr = err.await;
        let status = status?;
        if status.success() { written?; }
        let stdout = stdout.map_err(|_| WorkspaceError::WorkerFailed)??;
        let stderr = stderr.map_err(|_| WorkspaceError::WorkerFailed)??;
        Ok(GitOutput { status, stdout, stderr })
    }).await.map_err(|_| WorkspaceError::WorkerFailed)?
}
