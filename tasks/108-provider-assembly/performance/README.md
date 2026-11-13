# Provider decoding and Runtime assembly measurement

This manual probe compiles the actual provider SSE parser/decoder and Runtime
ModelDriver from each fixed source revision. Its only replacement is an in-memory
byte source in place of HTTP. It does not execute tools or contact a model service.

| Case | Fixed workload |
| --- | --- |
| Ordinary text | 16,384 frames, 16 text bytes each |
| Small tools | 4 interleaved calls, 128 payload bytes each |
| Large tools | 16 interleaved calls, 1 MiB payload each |
| Many tools | 128 interleaved calls, 4 KiB payload each |

Every call verifies the exact request, response parts, tool identities/order,
full JSON argument values, usage, replay metadata and text progress. Input and
expected-output identities must also match between each baseline/candidate pair.
No production decoder or assembler is copied into an alternative implementation.

Use Rust 1.90.0, the unchanged Cargo.lock and identical release build settings.
For each revision, run `prepare.py` through that environment:

```sh
python3 tasks/108-provider-assembly/performance/prepare.py \
  --root . --revision BASELINE_COMMIT --out /tmp/task108-baseline
python3 tasks/108-provider-assembly/performance/prepare.py \
  --root . --revision PUBLISHED_COMMIT --out /tmp/task108-candidate
```

Preparation exports the exact Git revision, adds this one example, records the
source/probe/lock/binary hashes and builds it without executing the probe.
After the candidate is published to dev and all builds have stopped:

```sh
python3 tasks/108-provider-assembly/performance/measure.py \
  --baseline-build /tmp/task108-baseline/build.json \
  --candidate-build /tmp/task108-candidate/build.json \
  --published-revision PUBLISHED_COMMIT --rounds 6 --out /tmp/task108-results
```

The runner alternates AB/BA order and reverses case order. It keeps every process,
warmup, per-call duration and failure. One warmup per process is excluded from
timing. Full response comparison is outside the per-call timer; ordinary driver
safety checks and the small progress/request checks remain inside it.

Linux process VmRSS/VmHWM are recorded after fixture setup, warmup and measured
calls. They include fixtures, expected responses, allocator retention and runtime
state; they are not allocation counts or isolated operation memory measurements.
Final response serialization happens after the memory observation. These results
describe local stream processing, not HTTP latency, model speed, complete Host
turn latency, live-provider behavior or resource-capacity limits.

Raw measurements and local environment records belong in the external output
directory. The repository contains the reproducible measurement source only.
