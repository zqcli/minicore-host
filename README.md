# Windows Job exit investigation

This isolated branch contains a standard-library Python probe, not a Host
implementation or release. It investigates the native failure from Host
candidate `56a768c9175a6d7cfe7b5de986d9145049f5a727`, Actions run
`43055978426`: wrapper completion and a retained descendant process handle
disagreed on whether cleanup had ended.

The probe starts a suspended real Git Bash, assigns it to one exact Job before
resuming, and observes gated native descendants. It retains process handles,
checks exact Job membership, compares accounting/PID-list/zero-wait observations,
and waits for actual process signals before cleanup. Read order is explicit;
observations are not an atomic snapshot. Forced Job termination, direct process
termination and natural exit are compared with 0/64 MiB leaf allocations.

No production Host/Runtime interface or source is changed by this branch.
Its results are investigation evidence, not a passing Host platform gate.
