use minicore_host_runtime::tools::ToolSpec;
use serde_json::{Value, json};

/// The declaration used both by registration and by each bound implementation.
pub fn local_tool_specs() -> Vec<ToolSpec> {
    vec![
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
}
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
