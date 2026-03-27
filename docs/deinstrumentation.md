# Understanding Dynamic De-instrumentation

> [!NOTE]
> Ocular is currently in its very early development phase. The dynamic de-instrumentation engine and threshold heuristics are experimental and subject to rapid iteration.

When building a tracer for modern Python (3.11+), you inevitably collide with the Heisenberg Uncertainty Principle of software: **observing the code changes how the code behaves.** Dynamic de-instrumentation is Ocular's solution to this problem, allowing it to gather highly detailed, instruction-level metrics without permanently destroying Python's native execution speed.

## The Observer Effect: CPython Quickening

Modern CPython utilizes a "Specializing Adaptive Interpreter". When Python notices a loop running frequently, it hot-swaps slow, generic instructions for highly optimized, type-specific ones. For example, it will dynamically rewrite a generic `BINARY_OP` into a blazing fast `BINARY_OP_MULTIPLY_INT` or `BINARY_OP_ADD_INT`.

When you ask Python for full instruction-level tracing, CPython is forced to de-optimize the bytecode. It must insert `INSTRUMENTED_INSTRUCTION` or `INSTRUMENTED_JUMP` hooks between your opcodes so the Rust backend can see them. **It physically cannot use its internal fast-paths while being monitored.** This results in a massive performance penalty.

## The Solution: `sys.monitoring.DISABLE`

PEP 669 (`sys.monitoring`) provides a built-in escape hatch for this performance penalty: the `DISABLE` object. 

Ocular leverages this to perform **Adaptive Sampling**:
1. **Tracking Hits:** Ocular maintains a thread-local `HOT_OFFSETS` map (`HashMap<(usize, i32), u32>`) that increments every time a specific instruction or jump offset is executed.
2. **Threshold Crossing:** When an instruction's hit count exceeds the globally configured `DEINSTRUMENT_THRESHOLD` (e.g., 500), Ocular decides it has gathered enough statistical data about that micro-operation.
3. **Unhooking:** The Rust FFI callback dynamically returns the `DISABLE` object to the Python Virtual Machine.
4. **Re-optimization:** CPython instantly strips the instrumentation hook from that specific bytecode offset. The overhead drops to zero, and CPython is free to re-quicken and optimize the loop for the remainder of the application's lifecycle.

## Configuring the Trade-off

You control this behavior via the `deinstrument_threshold` parameter in `ocular.start_tracing()`. This choice represents a fundamental trade-off between **statistical profiling** and **complete chronological tracing**.

### `threshold > 0` (Statistical Profiling)
* **How it works:** Ocular watches a loop execute `N` times, calculates the average hardware cycles, logs the exact sequence of opcodes, and then shuts the sensor off.
* **Pros:** Near-zero (or even negative) overhead for long-running scripts. Captures highly accurate hardware cycle averages for hot loops.
* **Cons:** The loop will simply vanish from your Perfetto timeline export after the `N`th iteration. It is not a complete historical record.

### `threshold = 0` (Full Tracing)
* **How it works:** Bypasses the `HOT_OFFSETS` check entirely. The `DISABLE` object is never returned.
* **Pros:** Captures every single tick, jump, and iteration, even if the loop runs 10 million times. You get a flawless, gapless timeline for Chrome tracing.
* **Cons:** You pay the CPython interrupt tax and FFI boundary cost for all 10 million hits, resulting in extremely high overhead.

## State Management

Because CPython caches these `DISABLE` signals internally, Ocular must explicitly tell the interpreter to forget these silenced instructions between tracing sessions. When you call `ocular.stop_tracing()`, the Rust backend automatically invokes `sys.monitoring.restart_events()` to clear the cache and reset the VM state.
