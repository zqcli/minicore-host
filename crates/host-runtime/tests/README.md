# Internal execution tests

The unit suite retains the original model-driver protocol, retry, cancellation,
deadline, panic, port-settlement, text normalization, JSON nesting and tool-value
coverage. Test-only channels observe direct driver calls without introducing a
production runner task.

The integration targets cover:

| Target | Contract |
| --- | --- |
| `execution_contract.rs` | Native history formats, shared views, execution budgets, capability snapshots and prompt projection |
| `model_partial_contract.rs` | Direct model-call partials, request identity, usage without content, ambiguous usage, actual retry facts and observation isolation |
| `tool_batch_contract.rs` | Ordered tools, ordinary failures, policy denial, approval and input, cancellation, deadline scopes and accepted-answer retention |
| `model_port_contract.rs` | Provider-facing request, response and context contract |
| `model_error_contract.rs` | Delivery state and retry-hint construction |
| `tool_policy_interaction_contract.rs` | Policy and interaction values and answer validation |
| `tool_set_contract.rs` | Frozen tool registration, schemas and implementations |
| `p1_dto.rs` | Model/tool wire values, normalization and unrestricted payloads |
| `p1_value.rs` | Text and JSON validation without application capacity quotas |
| `provider_replay.rs` | Opaque replay, nesting safety and native wire compatibility |

Standalone loop controls, reports, watches, callback handoffs and mutable-config
contracts were removed with those interfaces. The corresponding Session execution,
input boundaries, history saving, interaction ownership and finalization are
verified in `host-core` and the existing Host integration targets.

Run `cargo test -p minicore-host-runtime --all-targets --locked` and
`cargo test -p minicore-host-runtime --doc --locked` from the workspace root.
All adapters in this suite are test doubles; provider HTTP coverage belongs to
`host-provider` and the Host integration targets.
