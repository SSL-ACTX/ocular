# Ocular Usage Guide

Ocular exposes a single, clean Python API to control its Rust tracing engine. Because tracing fundamentally alters how the Python Virtual Machine executes code, configuring Ocular correctly is a matter of choosing between **accuracy of timing**, **accuracy of execution paths**, and **system overhead**.

## Initialization

You control Ocular via `ocular.start_tracing()`. This function accepts three primary arguments:

```python
import ocular

ocular.start_tracing(
    mode="adaptive",          # "adaptive" or "precise"
    perfetto=True,            # Enable/disable Chrome Trace format export
    deinstrument_threshold=500, # Number of hits before dynamic unhooking
    exclude=None,             # Optional list of path/pattern filters to ignore
    include_only=None         # Optional list of path/pattern filters to instrument
)
```

### 1. Tracing Modes (`mode`)

The `mode` dictates how deeply Ocular hooks into the CPython eval loop.

* **`mode="precise"` (Instruction-Level Profiling):**
    * Registers callbacks for every single `INSTRUCTION` executed by the VM.
    * **Behavior:** Forces CPython to replace opcodes with `INSTRUMENTED_INSTRUCTION` hooks. 
    * **Result:** You capture exact, discrete hardware CPU cycles for every micro-operation. However, this completely disables CPython's native JIT/quickening optimizations, resulting in massive overhead (e.g., 3000%+).
* **`mode="adaptive"` (Block-Level Profiling):**
    * Registers callbacks only for `PY_START`, `PY_RETURN`, `JUMP`, and `BRANCH` events.
    * **Behavior:** Ocular observes execution by basic blocks rather than individual lines. 
    * **Result:** CPython is free to quicken and specialize opcodes inside the blocks (e.g., `BINARY_OP -> BINARY_OP_ADD_INT`). Hardware cycles are reported as an evenly distributed average across the block's instructions.

### 2. Statistical vs. Full Tracing (`deinstrument_threshold`)

This threshold controls Ocular's dynamic de-instrumentation engine. 

* **Statistical Profiling (`threshold > 0`):** * Ocular tracks the execution count of specific instruction offsets. 
    * Once a loop or instruction hits this threshold (e.g., 500 times), Ocular dynamically returns a `DISABLE` signal to the Python VM.
    * Overhead drops to nearly 0% for the remainder of the program lifecycle, making this ideal for long-running production applications.
* **Full Chronological Tracing (`threshold = 0`):**
    * Bypasses the de-instrumentation logic entirely.
    * Ocular will intercept and record every event for the entire duration of the script. 
    * This provides a flawless, gapless timeline but incurs continuous FFI overhead.

### 3. Excluding and Including Code Paths (`exclude`, `include_only`)

* `exclude` lets you skip tracing modules / filenames / function names by substring match.
  * Example: `exclude=["/usr", "<frozen", "_unpack_opargs"]` will avoid overhead from runtime and internal CPython dispatch logic.
* `include_only` acts as a whitelist: if set, only matching code paths are traced.
  * Example: `include_only=["tests/test2.py"]` keeps the instrumentation focus on your benchmark code.
* Internally, Ocular caches per-`code_ptr` decisions in a zero-allocation thread-local `CodeFilter` table, so this filter decision is cheap and not repeated for every instruction.

### 4. Telemetry Export (`perfetto`)

* When `perfetto=True` is passed, Ocular translates the raw hardware traces into the Chrome Perfetto trace format (`PerfettoEvent` structs). 
* This data is asynchronously streamed to a lock-free queue and flushed to `ocular_trace.json` by a background worker thread when tracing stops.

## Graceful Shutdown

Always ensure you call `stop_tracing()` at the end of your profiling window.

```python
# Unhooks all sys.monitoring events, clears the DISABLE cache, 
# and safely joins the Rust telemetry worker thread.
ocular.stop_tracing()
```

If you do not call `stop_tracing()`, the lock-free `EVENT_QUEUE` may not completely flush your final events to the JSON export.
