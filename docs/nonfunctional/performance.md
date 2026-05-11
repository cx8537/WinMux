# Performance

> Service Level Objectives, measurement methodology, and hot paths.

WinMux's performance targets are modest by terminal-emulator
standards: a single user, a handful of sessions, the local machine.
But there are real latency and throughput SLOs because typing into a
laggy terminal is intolerable.

The guiding principle: **measure first, optimize second.** We do not
preemptively micro-tune.

---

## SLOs

The targets that matter:

| Operation | Target | Note |
| --- | --- | --- |
| **Key-to-PTY latency** (p95) | 8 ms | From `keydown` to bytes on PTY input |
| **PTY-to-screen latency** (p95) | 16 ms | From PTY output read to pixel on screen |
| **Session spawn** (p95) | 500 ms | From `NewSession` to first shell prompt |
| **Reattach** (p95) | 200 ms | From `Attach` request to `Attached` response with snapshot |
| **Throughput** | 50 MiB/s | Sustained pipeline (synthetic) |
| **Server idle RAM** | < 80 MiB | 4 sessions, 16 panes, idle |
| **Server idle CPU** | < 1% | Same workload |
| **Tray idle RAM** | < 200 MiB | Tauri + WebView baseline |
| **Capacity** | 16 sessions, 64 panes | Comfortable upper bound |

These are not hard limits; they are the bar below which "feels
fast" stops being true.

p95 = 95th percentile. We measure across realistic workloads, not
worst-case adversarial inputs.

---

## Hot Paths

The places where performance matters most:

1. **Keypress → PTY input.**
   - `keydown` in xterm.js
   - keyboard manager state-machine resolution
   - `PtyInput` IPC message construction
   - JSON Lines serialization
   - Named Pipe write
   - Server pipe read
   - Server dispatcher routes to the pane
   - PTY writer mutex lock
   - `MasterPty::write`
2. **PTY output → screen.**
   - PTY reader (`spawn_blocking` task)
   - Bounded channel into the pane manager
   - `VirtualTerm::feed`
   - Scrollback append
   - Broadcast queue per client
   - JSON Lines serialization
   - Named Pipe write
   - Client read loop
   - JSON Lines deserialization
   - xterm.js `write()`
   - WebGL render
3. **Snapshot on attach.**
   - `VirtualTerm::snapshot` (serialize grid to escape bytes)
   - Pack into `Attached` message
   - Send

Code in these paths is annotated with `// HOT PATH:` so it's
obvious to subsequent maintainers.

---

## Anti-patterns

What hot paths must not do:

- Allocate per-byte or per-cell.
- Take a Mutex unnecessarily.
- Hold a Mutex while awaiting.
- Copy buffers more than once (raw bytes → IPC frame).
- Run a regex on PTY content.
- Send a separate IPC message per byte.
- Use `format!` macros for log messages at INFO/DEBUG level in tight
  loops (use `tracing` structured fields).
- Re-render the React tree on every byte (xterm.js manages its
  canvas; React only renders pane structure).

---

## Measurement

### Tools

- **`criterion`** for Rust benchmarks. Reports include throughput,
  latency, and regression detection vs a baseline.
- **`tracing-perfetto`** for trace exports. Pulled in only with
  `--features=perfetto`, off by default.
- **Manual timing** in dev: `tracing::trace!("foo took {ms}", ...)`
  during exploration.
- **Sidabari's bench harness** as inspiration for E2E timing.

### When to bench

- Adopted from M2 onwards. The PoC and MVP focus on correctness.
- A PR that changes hot-path code must include before/after numbers
  in the description.
- Nightly CI runs the bench suite and posts results as a comment on
  HEAD. Regressions are noted but not a hard fail (false positives
  are common in CI).

### Benchmark coverage

The benches that exist (from M2):

1. **`bench_keypress_to_pty`** — synthetic; measures
   `PtyInput`-to-pipe-write round-trip time.
2. **`bench_pty_to_snapshot`** — feeds a stream of mixed printable
   and escape sequence bytes; measures `VirtualTerm::feed` rate.
3. **`bench_snapshot`** — fills a `VirtualTerm` and measures
   `snapshot()` time at various screen sizes.
4. **`bench_scrollback_append`** — measures scrollback ring-buffer
   append rate with and without disk mirror.
5. **`bench_protocol_codec`** — `serde_json` encode/decode for the
   message types.
6. **`bench_pipe_throughput`** — Named Pipe raw write/read rate, as a
   ceiling check.

---

## Memory

### Server

Baseline (4 sessions, 16 panes, idle, default scrollback):

- Per pane: ~1.5 MiB (`alacritty_terminal` grid + 10k-line ring
  buffer with average line cost).
- Per session metadata: < 1 KiB.
- IPC buffers: bounded per channel.
- Total: < 80 MiB target.

Growth with scrollback:

- 100k lines per pane → ~15 MiB per pane.
- 16 panes × 100k lines → ~240 MiB.
  → This is why the default cap is 10k, configurable up.

### Tray (Tauri + WebView)

WebView2 has a non-trivial baseline (~120 MiB). xterm.js with WebGL
adds ~30 MiB for the renderer. Tauri's bridge is small.

Target: < 200 MiB idle, < 350 MiB with several active terminals.
This is acceptable for a daily-driver dev tool. We do not aim for the
2 MiB native-toolkit footprint that some other terminals advertise —
the tradeoff is the freedom Tauri gives us in UI development.

---

## CPU

### Idle

`tracing` background writer + Tokio scheduler + the periodic
session-save tick. Should be < 1% on a modern laptop CPU. If it's
higher, something is busy-spinning.

### Active

Bursts during heavy output (e.g., `cat large.txt`). Most of the work
is `alacritty_terminal::feed`. We have headroom: a single core can
comfortably parse hundreds of MiB/s of typical terminal output.

---

## Throughput

Synthetic test: pipe `seq 1 1000000` (or PowerShell `1..1000000`)
through to a pane and measure end-to-end. Target: 50 MiB/s sustained
without dropping frames.

Realistic test: build a large project (`cargo build`) and read its
output. The bottleneck here is `cargo`, not WinMux.

---

## Latency Sensitivity

Key-to-PTY latency below 8 ms p95 means a typist won't notice
WinMux specifically. For comparison:

- Local terminal emulators measured well are in the 5–10 ms range.
- Going through ssh or tmux adds further delay.
- WinMux's IPC adds: serialize (negligible), pipe write/read (small
  ms), dispatcher routing (negligible). The Tauri WebView event
  loop is the biggest contributor on this side.

---

## When to Worry About Performance

Symptoms that warrant a bench-and-investigate pass:

- Visible typing lag.
- High CPU during idle.
- High RAM growth over time (likely a leak in scrollback or virtual
  terminal lifecycle).
- Tray UI animation jank.
- Slow startup of the tray (> 2 s on a modern PC).

When investigating:

1. Profile, don't guess. `tracing` events with timing fields, plus
   either `criterion` or `perf`/`tracy`.
2. Identify the actual bottleneck.
3. Fix the algorithm before tuning constants.
4. Include before/after numbers in the PR description.

---

## What We Don't Optimize For

- Cold-start time below 200 ms. Tauri + WebView precludes this.
- Memory < 50 MiB. The WebView alone exceeds it.
- 144 fps terminal rendering. 60 fps is the goal; xterm.js WebGL
  meets it on modern hardware.
- Pathological input (e.g., `find /` with millions of lines piped
  in). The system stays correct, but bounded queues will drop or
  disconnect a slow consumer.

---

## Related Docs

- IPC framing and bounded queues → [`../spec/01-ipc-protocol.md`](../spec/01-ipc-protocol.md)
- PTY and virtual terminal → [`../spec/02-pty-and-terminal.md`](../spec/02-pty-and-terminal.md)
- Stability and resource management → [`stability.md`](stability.md)
