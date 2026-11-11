use std::{
    fs,
    io::{Read, Seek, SeekFrom},
};

use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::{
    QueryContext, Result, Workspace, WorkspaceError,
    context::blocking,
    workspace::{normalize, path_text, read_bytes, relative_path_text},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub kind: FileKind,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesRequest {
    /// Empty string selects the bound root.
    pub directory: String,
    pub recursive: bool,
    pub query: Option<String>,
    pub cursor: Option<FilesCursor>,
    /// Requested page size, not a Host-imposed maximum.
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesCursor {
    pub scope: String,
    pub after_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FilesResult {
    pub entries: Vec<FileEntry>,
    pub next_cursor: Option<FilesCursor>,
    pub truncated: bool,
}

/// UTF-8 byte offsets, [start, end). `end: None` selects the current file end.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    pub path: String,
    pub start: u64,
    pub end: Option<u64>,
    pub max_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadResult {
    pub path: String,
    pub start: u64,
    pub end: u64,
    pub text: String,
    /// More bytes remain inside the caller's requested range.
    pub has_more: bool,
    pub file_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    /// A file or directory relative to the bound root; empty selects the root.
    pub path: String,
    pub query: String,
    pub case_sensitive: bool,
    /// Number of matching lines requested. All matches on a returned line are included.
    pub max_matches: usize,
    pub cursor: Option<SearchCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchCursor {
    pub scope: String,
    pub path: String,
    /// Continue after this one-based matching line.
    pub line_number: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub path: String,
    pub line_number: usize,
    /// Original line text without its LF or CRLF terminator.
    pub line_text: String,
    pub match_byte_ranges: Vec<ByteRange>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    pub next_cursor: Option<SearchCursor>,
    /// Binary/non-UTF-8 files observed before this page ended, not silently searched as text.
    pub skipped_binary_files: Vec<String>,
}

impl Workspace {
    pub async fn files(&self, request: FilesRequest, context: QueryContext) -> Result<FilesResult> {
        let workspace = self.clone();
        blocking(context, move |context| {
            if request.limit == 0 {
                return Err(WorkspaceError::InvalidRange);
            }
            let directory = normalize(&request.directory, true)?;
            let scope = workspace.scope(&[
                "files",
                path_text(&directory)?,
                if request.recursive {
                    "recursive"
                } else {
                    "one_level"
                },
                request.query.as_deref().unwrap_or(""),
            ]);
            if let Some(cursor) = &request.cursor {
                normalize(&cursor.after_path, false)?;
                if cursor.scope != scope {
                    return Err(WorkspaceError::InvalidCursor);
                }
            }
            let entries = workspace.walk(&request.directory, request.recursive, context)?;
            let mut selected = entries.into_iter().filter(|entry| {
                request
                    .query
                    .as_ref()
                    .is_none_or(|query| entry.path.contains(query))
                    && request
                        .cursor
                        .as_ref()
                        .is_none_or(|cursor| entry.path > cursor.after_path)
            });
            let entries: Vec<_> = selected.by_ref().take(request.limit).collect();
            let truncated = selected.next().is_some();
            let next_cursor = truncated.then(|| FilesCursor {
                scope,
                after_path: entries.last().expect("positive limit").path.clone(),
            });
            Ok(FilesResult {
                entries,
                next_cursor,
                truncated,
            })
        })
        .await
    }

    pub async fn read(&self, request: ReadRequest, context: QueryContext) -> Result<ReadResult> {
        let workspace = self.clone();
        blocking(context, move |context| {
            let mut file = workspace.file(&request.path)?;
            let before = file.metadata()?;
            let end = request.end.unwrap_or(before.len()).min(before.len());
            if request.start > end || (request.start < end && request.max_bytes == 0) {
                return Err(WorkspaceError::InvalidRange);
            }
            for offset in [request.start, end] {
                if offset < before.len() {
                    file.seek(SeekFrom::Start(offset))?;
                    let mut byte = [0_u8];
                    file.read_exact(&mut byte)?;
                    if byte[0] & 0xc0 == 0x80 {
                        return Err(WorkspaceError::InvalidRange);
                    }
                }
            }
            file.seek(SeekFrom::Start(request.start))?;
            let amount = (end - request.start).min(request.max_bytes as u64);
            let mut bytes = read_bytes((&mut file).take(amount), context)?;
            let after = file.metadata()?;
            if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
                return Err(WorkspaceError::Changed);
            }
            if bytes.len() as u64 != amount {
                return Err(WorkspaceError::Changed);
            }
            if bytes.contains(&0) {
                return Err(WorkspaceError::Binary);
            }
            if let Err(error) = std::str::from_utf8(&bytes) {
                if error.error_len().is_none() && request.start + amount < end {
                    bytes.truncate(error.valid_up_to());
                    if bytes.is_empty() {
                        return Err(WorkspaceError::InvalidRange);
                    }
                } else {
                    return Err(WorkspaceError::Binary);
                }
            }
            let end_returned = request.start + bytes.len() as u64;
            Ok(ReadResult {
                path: relative_path_text(&normalize(&request.path, false)?)?,
                start: request.start,
                end: end_returned,
                text: String::from_utf8(bytes).map_err(|_| WorkspaceError::Binary)?,
                has_more: end_returned < end,
                file_bytes: after.len(),
            })
        })
        .await
    }

    pub async fn search(
        &self,
        request: SearchRequest,
        context: QueryContext,
    ) -> Result<SearchResult> {
        let workspace = self.clone();
        blocking(context, move |context| {
            if request.max_matches == 0
                || request.query.is_empty()
                || request.query.contains(['\n', '\r', '\0'])
            {
                return Err(WorkspaceError::InvalidRange);
            }
            let path = normalize(&request.path, true)?;
            let scope = workspace.scope(&[
                "search",
                path_text(&path)?,
                &request.query,
                if request.case_sensitive {
                    "sensitive"
                } else {
                    "insensitive"
                },
            ]);
            if let Some(cursor) = &request.cursor {
                normalize(&cursor.path, false)?;
                if cursor.scope != scope || cursor.line_number == 0 {
                    return Err(WorkspaceError::InvalidCursor);
                }
            }
            let pattern = RegexBuilder::new(&regex::escape(&request.query))
                .case_insensitive(!request.case_sensitive)
                .size_limit(usize::MAX)
                .build()
                .map_err(|_| WorkspaceError::InvalidRange)?;
            let resolved = workspace.resolve(&request.path, true)?;
            let entries = if resolved.is_dir() {
                workspace.walk(&request.path, true, context)?
            } else if resolved.is_file() {
                vec![FileEntry {
                    path: relative_path_text(&path)?,
                    kind: FileKind::File,
                    size: None,
                }]
            } else {
                return Err(WorkspaceError::NotFile);
            };
            let mut matches: Vec<SearchMatch> = Vec::new();
            let mut skipped_binary_files = Vec::new();
            for entry in entries
                .into_iter()
                .filter(|entry| entry.kind == FileKind::File)
            {
                context.check()?;
                if request
                    .cursor
                    .as_ref()
                    .is_some_and(|cursor| entry.path < cursor.path)
                {
                    continue;
                }
                let bytes = read_bytes(workspace.file(&entry.path)?, context)?;
                let text = match std::str::from_utf8(&bytes) {
                    Ok(text) if !bytes.contains(&0) => text,
                    _ => {
                        skipped_binary_files.push(entry.path);
                        continue;
                    }
                };
                for (index, line) in text.lines().enumerate() {
                    context.check()?;
                    let line_number = index + 1;
                    if request.cursor.as_ref().is_some_and(|cursor| {
                        entry.path == cursor.path && line_number <= cursor.line_number
                    }) {
                        continue;
                    }
                    let ranges: Vec<_> = pattern
                        .find_iter(line)
                        .map(|found| ByteRange {
                            start: found.start(),
                            end: found.end(),
                        })
                        .collect();
                    if ranges.is_empty() {
                        continue;
                    }
                    if matches.len() == request.max_matches {
                        let last = matches.last().expect("positive max_matches");
                        return Ok(SearchResult {
                            next_cursor: Some(SearchCursor {
                                scope,
                                path: last.path.clone(),
                                line_number: last.line_number,
                            }),
                            matches,
                            skipped_binary_files,
                        });
                    }
                    matches.push(SearchMatch {
                        path: entry.path.clone(),
                        line_number,
                        line_text: line.to_owned(),
                        match_byte_ranges: ranges,
                    });
                }
            }
            Ok(SearchResult {
                matches,
                next_cursor: None,
                skipped_binary_files,
            })
        })
        .await
    }

    fn walk(
        &self,
        directory: &str,
        recursive: bool,
        context: &QueryContext,
    ) -> Result<Vec<FileEntry>> {
        let directory = self.directory(directory)?;
        let mut builder = WalkBuilder::new(directory);
        builder
            .current_dir(self.root())
            .hidden(false)
            .parents(false)
            .git_global(false)
            .require_git(false)
            .follow_links(false)
            .max_depth((!recursive).then_some(1))
            .filter_entry(|entry| {
                #[cfg(any(windows, target_os = "macos"))]
                {
                    !entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(".git"))
                }
                #[cfg(not(any(windows, target_os = "macos")))]
                {
                    entry.file_name() != ".git"
                }
            });
        let mut entries = Vec::new();
        for entry in builder.build() {
            context.check()?;
            let entry = entry.map_err(|error| WorkspaceError::Io(std::io::Error::other(error)))?;
            if entry.depth() == 0 {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(self.root())
                .map_err(|_| WorkspaceError::OutsideWorkspace)?;
            let metadata = fs::symlink_metadata(entry.path())?;
            let kind = if metadata.is_symlink() {
                FileKind::Symlink
            } else if metadata.is_file() {
                FileKind::File
            } else if metadata.is_dir() {
                FileKind::Directory
            } else {
                FileKind::Other
            };
            entries.push(FileEntry {
                path: relative_path_text(relative)?,
                kind,
                size: metadata.is_file().then_some(metadata.len()),
            });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }
}
