<div align="center">

![Ocular Banner](https://svg-banners.vercel.app/api?type=luminance&text1=Ocular%20%F0%9F%8E%AF&width=800&height=200&color=8A2BE2)

![Version](https://img.shields.io/badge/version-0.1.0-blue.svg?style=for-the-badge)
![Language](https://img.shields.io/badge/language-Rust%20%7C%20Python-orange.svg?style=for-the-badge&logo=rust)
![License](https://img.shields.io/badge/license-AGPL_3.0-green.svg?style=for-the-badge)

**Ultra-low overhead PEP 669 instruction-level tracer and telemetry engine for Python.**

[Architecture](docs/architecture.md) • [Usage Guide](docs/usage.md) • [Telemetry & Perfetto](docs/telemetry.md) • [Benchmarks](docs/benchmarks.md)

</div>

---

## Overview

**Ocular** is a high-performance tracing and profiling runtime built in Rust, designed exclusively for Python 3.12+ via the PEP 669 (`sys.monitoring`) API. 

It bridges the gap between deep, instruction-level visibility and production-grade performance. By utilizing lock-free zero-allocation pipelines and hardware-level cycle counting, Ocular observes Python's execution state without destroying its speed. 

Ocular operates in two primary modes:
- **Statistical Profiling (Adaptive):** Dynamically unhooks from hot loops once a statistical threshold is reached, allowing Python's Specializing Adaptive Interpreter to resume native optimizations (achieving near-zero or even *negative* overhead).
- **Chronological Tracing (Full):** Captures a flawless, gapless timeline of every opcode, jump, and function call for deep offline analysis.

> [!NOTE]
> Ocular strictly requires Python 3.12 or higher to leverage the `sys.monitoring` API. 

---

## Core Capabilities

- **Hardware Cycle Accuracy:** Bypasses expensive OS system clock calls, using hardware Time Stamp Counters (TSC) for ultra-low-overhead, nanosecond-precision execution timing.
- **Zero-Allocation FFI Pipeline:** Uses crossbeam lock-free queues, object pools, and batched FFI transitions to completely eliminate heap allocation in the tracing hot path.
- **Dynamic De-instrumentation:** Automatically detects hot loops and returns `DISABLE` to the Python VM after a configurable threshold, gracefully stepping out of the way of CPython's JIT/quickening optimizations.
- **Hot Trace Disassembly:** Maps runtime traces back to specific file lines and base-to-specialized opcode transitions (e.g., `BINARY_SUBSCR -> BINARY_SUBSCR_LIST_INT`).
- **Perfetto Export:** Natively generates Chrome Perfetto (`.json`) traces for visualizing function execution, loop micro-ops, and call stack hierarchies.

---

## Quick Start

### Installation

Ensure you have Rust and a Python 3.12+ virtual environment active.

```bash
pip install maturin
maturin develop --release
````

### Basic Example (Python)

```python
import ocular
import time

def hot_loop_workload(limit):
    total = 0
    for i in range(limit):
        total += i * 2
    return total

# Start Ocular with adaptive de-instrumentation (threshold=500)
# This profiles the first 500 iterations, then drops overhead to zero.
ocular.start_tracing(mode="precise", perfetto=True, deinstrument_threshold=500)

start = time.time()
result = hot_loop_workload(1_000_000)
print(f"Elapsed: {time.time() - start:.4f}s")

# Stops tracing and flushes the telemetry worker queue
ocular.stop_tracing()
```

-----

## Learn More

  - [Full Architecture Reference](docs/architecture.md)
  - [Usage Examples & API Guide](docs/usage.md)
  - [Understanding Dynamic De-instrumentation](docs/deinstrumentation.md)
  - [Exporting and Viewing Perfetto Timelines](docs/telemetry.md)

-----

## Disclaimer

> [!IMPORTANT]
> **Production Status:** Ocular is currently in **Alpha**.
>
> **Performance (v0.1.0):**
>
>   - **Event Processing:** ~220ns per PEP 669 event transition (FFI + TSC read + queue push).
>   - **Full Tracing Overhead (threshold=0):** ~300% - 700% depending on instruction density, blocking Python's internal quickening.
>   - **Adaptive Overhead (threshold=500):** Near 0% (or mildly negative due to cache warming behaviors), as Ocular dynamically unhooks and allows CPython to fully optimize the hot loops.
>   - **See more at:** [v0.1.0 Benchmarks](docs/benchmarks.md)

-----

<div align="center">

**Author:** Seuriin ([SSL-ACTX](https://github.com/SSL-ACTX))

</div>
