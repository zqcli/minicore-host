# Preserve the production approval review

The earlier independent review approved source
`0fcf999183084a6a310d721996c71b264afe7a53`, integrated as `73dad22` and
published as `982951341a77e3cddc005513e9bcb67a54793f84`. It reported 12 independent
cases and 50 repeated executions, including actual binary baseline-red/fixed-green
checks and external tools named read/agent. Producer checks reported 211 affected
cases; these counts are distinct, not an aggregate workspace gate.

The production policy freezes Ask(default), Auto or ReadOnly per actual Turn.
Only recognized builtin origins receive read/inspection/wait exemptions; generic
registered tools are treated as mutating even if their names match builtins.
The original interaction and cancellation owner paths remain authoritative.
The migration guide explains old configurations that omit approval and explicit
Auto fixtures. No old Agent compatibility baseline or OS sandbox is claimed.

This summary preserves the reported review scope after the second replacement.
Original raw logs and the uncommitted 12-probe artifact were lost; their prior
results are not fresh validation. Published production source and committed
regressions remain available and must be rerun in the final combined gate.
