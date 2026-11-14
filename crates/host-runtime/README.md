# Internal Host execution

This workspace library is derived from minicore-runtime 0.10.0 at
[`3aa47629929906832dcb309afb7d242c1ae3e53f`](https://github.com/zqcli/minicore-runtime/tree/3aa47629929906832dcb309afb7d242c1ae3e53f).
The source declares the same MIT OR Apache-2.0 license as this workspace.

Session owns the complete Turn and its history. This library provides the
model-call and tool execution mechanisms and their shared native values.
Provider HTTP/protocol adaptation remains in host-provider; actual side effects
and resource cleanup remain in the concrete tool implementations.

`ModelDriver` borrows the already selected model and directly awaits one logical
request. It owns protocol assembly, delivery-aware retry, deadline/cancellation,
and panic isolation. Its returned report contains actual attempt count, retry
wait, response or failure, usage, and partial output; observations do not supply
these facts.

`ModelCallParams` carries the owner's optional execution deadline. The driver
combines it with the configured model timeout once per logical request and gives
each provider attempt a `ModelCallContext` with that same effective deadline.
An absent execution deadline does not create an artificial Turn deadline.

`run_tool_batch` awaits the validated response's tools in order under the same
frozen execution snapshot. The Session registers and settles its interactions
through `ToolBatchHost`. The batch returns ordered results and any stop reason;
the concrete tool owners still confirm actual cleanup before the Session finishes.

`ToolSpec` shares immutable, checked declarations. A selected `ToolSet` freezes
the exact declaration read from each implementation; checked factory binding
compares that same value with its registration. Execution wrappers map the bound
model or tools while retaining those declarations and the validated selection.
Policy requests borrow read-only invocation data, and execution consumes the
owned arguments. These internal changes preserve the serialized value formats.

There is no spawned loop, independent control plane, input/history callback, or
accumulated history report. `LoopId` remains a native correlation value so existing
history records keep their format. See [SPEC 4.10.3](../../SPEC.md) and
[Task 106](../../tasks/106-host-runtime/TASK.md).
