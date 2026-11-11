# Task 105: native multi-platform CI

## Authorized scope

The owner requested Host CI like minicore-agent with Runtime compiled into Host.
Host/Runtime interfaces, Runtime revision and version 0.9.20 remain unchanged.
Baseline dev is `6ea85fda737635e26b9d7cad17c3df1e54911bd9`.

## Isolated Windows prerequisite probe

This `ci/windows-storage-probe` topic contains only a standalone safe Rust probe
and its Windows Actions workflow. It does not enable or claim a Windows storage
implementation. The existing Linux storage code explicitly rejects Windows;
directory synchronization must be investigated without replacing it with a no-op.

The probe compiles directly with Rust 1.90.0, opens fresh directories using six
Windows access modes, and reports actual `File::sync_all` results after creation,
rename and deletion, plus results for canonical ancestors. Successful calls are
API observations, not a power-loss test. Each failed call retains its OS code.

This separate topic lets the native API run before implementing a backend and
does not change `dev`. Results and resulting implementation decisions belong in
the completed Task 105 record on the implementation topic.

Workers are requested with gpt-6-astra and max reasoning, following AGENTS.md;
those are requested settings, not an independent runtime attestation.
