use minicore_host_runtime::tools::ToolSpec;
use serde_json::{Value, json};
use std::sync::LazyLock;

/// The declaration used both by registration and by each bound implementation.
pub fn local_tool_specs() -> Vec<ToolSpec> {
    LOCAL_TOOL_SPECS.to_vec()
}

pub(crate) fn local_tool_spec(name: &str) -> Option<ToolSpec> {
    LOCAL_TOOL_SPECS
        .iter()
        .find(|spec| spec.name().as_str() == name)
        .cloned()
}

static LOCAL_TOOL_SPECS: LazyLock<[ToolSpec; 5]> = LazyLock::new(|| {
    [
        spec(
            "read",
            "Read an explicit line range from a UTF-8 workspace file, or an explicit sorted range of directory entries. No hidden output truncation.",
            json!({
                "path":{"type":"string"},
                "offset":{"type":"integer","minimum":1,"default":1},
                "limit":{"type":"integer","minimum":1,"description":"Number of lines or directory entries requested."}
            }),
            &["path", "limit"],
        ),
        spec(
            "edit",
            "Replace exact literal UTF-8 text in one existing workspace file. Multiple matches require replace_all. The replacement is atomic; conflicts are reported without a write.",
            json!({
                "path":{"type":"string","minLength":1},
                "old_text":{"type":"string","minLength":1},
                "new_text":{"type":"string"},
                "replace_all":{"type":"boolean","default":false}
            }),
            &["path", "old_text", "new_text"],
        ),
        spec(
            "write",
            "Atomically write UTF-8 content to a workspace-relative file, creating missing parent directories. Returns the actual write outcome.",
            json!({
                "path":{"type":"string","minLength":1},
                "content":{"type":"string"}
            }),
            &["path", "content"],
        ),
        spec(
            "apply_patch",
            "Apply a complete exact patch to one existing UTF-8 workspace file. Accepts a single-file unified diff (with optional matching headers) or one Codex *** Begin Patch / *** Update File / *** End Patch envelope. Preserves source newline semantics. Rejects Add File, Delete File, Move to, multifile patches and ambiguous matches; no fuzzy matching. All hunks must validate before atomic replacement.",
            json!({
                "path":{"type":"string","minLength":1,"description":"Workspace-relative path of the existing file; patch headers must match."},
                "patch":{"type":"string","minLength":1,"description":"Exact unified diff or single Update File envelope. Example: *** Begin Patch\n*** Update File: value.txt\n@@\n-old\n+new\n*** End Patch"}
            }),
            &["path", "patch"],
        ),
        spec(
            "bash",
            "Run Bash in the bound workspace (Unix Bash or Windows Git Bash). Returns byte-exact stdout and stderr plus the actual exit status. Cancellation stops and waits for the process group or job. Arbitrary shell file changes are unknown, not inferred from Git.",
            json!({
                "command":{"type":"string","minLength":1},
                "cwd":{"type":"string","default":".","description":"A workspace-relative directory."},
                "timeout_seconds":{"type":"integer","minimum":1,"description":"Optional shorter deadline; omission uses the Runtime tool deadline."}
            }),
            &["command"],
        ),
    ]
});
fn spec(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolSpec {
    ToolSpec::new(
        name.parse().expect("static tool name"),
        description,
        json!({
            "type":"object","properties":properties,"required":required,"additionalProperties":false
        }),
    )
    .expect("static local tool declaration")
}
