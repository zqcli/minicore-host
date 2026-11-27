# Task 123: Markdown instruction files

Baseline: Host `dev` `a900fc45d19e0f104e3b9bbc4502642dd7788e99`
(0.12.16, CI #86). The runtime and package version are unchanged.

Host, Agent and Role declarations use `instructions` or `instructions_file`.
The shared loader in `crates/host-config/src/directory.rs` expands relative
UTF-8 Markdown paths within the configuration root into the existing prompt
layers. The Host's previous `framework_instructions` name is retired.
SPEC advances to 4.11.7; see `docs/configuration.md` for the existing-file
migration. Session configuration keeps its copied values and no file paths
are persisted or sent to Runtime.

Local Linux verification (Rust 1.90.0, writable temporary HOME):
- `cargo fmt --all -- --check` and `git diff --check`: pass.
- `cargo test -p minicore-host-config -p minicore-host-core --test directories --test configuration_directory --test native_configuration_reads --locked`:
  19 passed. Covers all layers, sibling and same-directory paths, invalid
  declarations, and a real Host restart with unchanged then updated Markdown.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: pass.
- Cross-platform tests and package builds are tracked by CI for this commit.
  No external model, TUI, or Web validation was performed.
