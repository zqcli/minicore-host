use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
};

use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, process::Command};

use crate::{
    QueryContext, Result, Workspace, WorkspaceError,
    context::blocking,
    workspace::{normalize, path_text},
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
                return Ok(None);
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
        let root = PathBuf::from(text);
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

    fn relative(&self, path: &str) -> Result<String> {
        let path = normalize(path, false)?;
        let relative = path
            .strip_prefix(&self.prefix)
            .map_err(|_| WorkspaceError::OutsideWorkspace)?;
        Ok(path_text(relative)?.to_owned())
    }

    fn repo_path(&self, path: &str) -> Result<String> {
        Ok(path_text(&self.prefix.join(normalize(path, false)?))?.to_owned())
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
    context.check()?;
    let root = root.to_owned();
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let context = context.clone();
    // This small process owner outlives an abandoned caller. It always drains
    // output and reaps the actual child, also after caller cancellation.
    tokio::spawn(async move {
        context.check()?;
        let mut command = Command::new("git");
        command.args(["--no-pager", "-c", "core.fsmonitor=false"])
            .args(args).current_dir(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true)
            .env("LC_ALL", "C").env("GIT_OPTIONAL_LOCKS", "0").env("GIT_LITERAL_PATHSPECS", "1")
            .env("GIT_NO_LAZY_FETCH", "1").env("GIT_TERMINAL_PROMPT", "0");
        for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"] { command.env_remove(name); }
        let mut child = command.spawn().map_err(WorkspaceError::GitUnavailable)?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let out = tokio::spawn(async move { let mut bytes = Vec::new(); stdout.read_to_end(&mut bytes).await.map(|_| bytes) });
        let err = tokio::spawn(async move { let mut bytes = Vec::new(); stderr.read_to_end(&mut bytes).await.map(|_| bytes) });
        let status = tokio::select! {
            biased;
            reason = context.stopped() => { child.kill().await.map_err(WorkspaceError::Io).and(Err(reason)) },
            status = child.wait() => status.map_err(WorkspaceError::Io),
        };
        let stdout = out.await.map_err(|_| WorkspaceError::WorkerFailed)??;
        let stderr = err.await.map_err(|_| WorkspaceError::WorkerFailed)??;
        Ok(GitOutput { status: status?, stdout, stderr })
    }).await.map_err(|_| WorkspaceError::WorkerFailed)?
}
