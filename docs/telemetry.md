# Ocular Telemetry & Perfetto Integration

> [!NOTE]
> Ocular is currently in its very early development phase (literally a day old!). The telemetry formats, export mechanisms, and analysis features are experimental and subject to rapid iteration.

Ocular’s telemetry engine is built to aggregate millions of instruction-level execution events without stalling the Python Global Interpreter Lock (GIL). To achieve this, Ocular shifts the heavy lifting of trace reconstruction, hardware cycle averaging, and JSON serialization to a dedicated background Rust thread.

## The Telemetry Pipeline

When `sys.monitoring` fires an event, Ocular's FFI callbacks immediately push a lightweight `TraceEvent` enum payload to a lock-free queue (`EVENT_QUEUE`) and return control to CPython. The `telemetry_worker` thread continuously drains this queue in the background.

The worker performs several critical tasks entirely offline from the hot path:
* **Symbol Resolution & Caching:** It lazily interrogates Python's `dis` module to map raw instruction offsets to human-readable opcode names. This metadata is cached in a `CodeMeta` registry to avoid repeated lookups.
* **Call Stack Tracking:** It maintains an internal mirror of the Python call stack to accurately pair function entries (`PyStart`) with function exits (`PyReturn`).
* **Cycle Averaging:** It calculates the hardware Time Stamp Counter (TSC) clock cycles elapsed between instructions. It averages these cycles across loop iterations to provide highly stable, nanosecond-accurate performance metrics.
* **Zero-Allocation Recycling:** As batches of events are processed, the worker pushes the empty vectors to a `FREE_QUEUE` so the frontend callbacks can reuse them, completely eliminating heap allocation during tracing.

## Hot Trace Disassembly

At the end of a tracing session, Ocular analyzes its internal `TraceStats` map to identify the hottest execution paths (loops). It then outputs a highly detailed disassembly directly to the console:

```text
[Ocular] Trace Disassembly with Hardware CPU Cycles (Base -> Specialized):
  [000] offset 94 : FOR_ITER -> FOR_ITER_RANGE                    | ~1567 cycles
  [001] offset 98 : STORE_FAST                                    | ~582 cycles
  [002] offset 104: STORE_SUBSCR -> STORE_SUBSCR_LIST_INT         | ~458 cycles
```

* **Base -> Specialized Transitions:** Because modern CPython uses a Specializing Adaptive Interpreter, Ocular tracks both the original opcode (e.g., `STORE_SUBSCR`) and the specialized, quickened opcode Python swapped in at runtime (e.g., `STORE_SUBSCR_LIST_INT`).
* **Cycle Latency:** Next to each micro-operation, you will see the hardware cycles consumed. If Ocular is running in `adaptive` mode, this represents the average block latency. If running in `precise` mode, this represents the exact instruction-to-instruction cycle cost.

## Perfetto Timeline Export

When tracing is initialized with `perfetto=True`, Ocular translates the raw hardware execution events into the open-source Chrome Trace Event Format. 

Once `ocular.stop_tracing()` is called, the telemetry worker flushes this data into a file named `ocular_trace.json` in your current working directory.

### Viewing the Trace

1. Open a Chromium-based browser (Chrome, Edge, Brave).
2. Navigate to `ui.perfetto.dev` or `chrome://tracing`.
3. Drag and drop the `ocular_trace.json` file into the viewer.

### Perfetto Event Types

Ocular structures the JSON payload using `PerfettoEvent` structs. The timeline represents the following distinct phases:

* **Function Boundaries (`ph: "B"` and `ph: "E"`):** Function invocations are tracked via `PyStart` and `PyReturn` events. These form the macro-level flame graph of your application's call tree. Ocular attaches the raw memory `code_ptr` as an argument to these events to help disambiguate calls.
* **Loop Analytics (`ph: "i"`):** When a jump backwards is detected (signifying a loop body repeating), Ocular emits an Instant event categorized under "loop". This event embeds the number of micro-ops (`uOps`) executed in the loop body and the total `Hardware Cycles` consumed by that specific iteration.
