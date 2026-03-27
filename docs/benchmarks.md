# Ocular Performance Benchmarks

> [!NOTE]
> These benchmarks were captured on an `x86_64` Linux environment using Python 3.13.7. Results may vary based on CPU architecture (TSC frequency) and specific CPython micro-releases.
> Using a single-core, 16yo laptop!

Ocular is designed to provide the best possible performance for a PEP 669-based tracer. Below is a breakdown of the overhead costs associated with different tracing strategies.

## Micro-Benchmark: Event Latency

The fundamental cost of tracing is the "Tax per Event"—the time it takes for CPython to halt, cross the FFI boundary into Rust, and for Ocular to process the data.

| Operation | Latency (Approx) | Description |
| :--- | :--- | :--- |
| **Empty Python Call** | ~50-80ns | Baseline CPython overhead for a no-op function call. |
| **Ocular Event Hook** | **~221ns** | Total time for FFI transition + Hardware TSC read + Batch queue push. |
| **Telemetry Processing** | Asynchronous | Reconstructing traces and symbol resolution happens on a background thread. |

## Macro-Benchmark: `primes_sieve` (20k limit)

This benchmark measures the overhead of running a computationally intensive prime number sieve.

### 1. No Tracing (Baseline)
* **Execution Time:** ~0.0075s.
* **Optimization:** Full CPython quickening and specialization enabled.

### 2. Adaptive Mode (`threshold=500`)
* **Execution Time:** ~0.0046s to ~0.0076s.
* **Overhead:** **-41% to +7%**.
* **Why it's so fast:** Ocular gathers 500 samples per instruction/jump and then returns `sys.monitoring.DISABLE`. CPython then strips the instrumentation and applies full optimizations. The "negative" overhead often seen is due to the baseline run lacking the same warm-up characteristics as the traced run.

### 3. Full Tracing (`threshold=0`)
* **Execution Time:** ~0.0650s.
* **Overhead:** **~737%**.
* **Total Events:** ~178,000 events processed.
* **Impact:** Prevents CPython from using specialized opcodes (e.g., `BINARY_OP_ADD_INT`) because the tracer must see every generic instruction.

### 4. Precise Mode (`mode="precise"`, `threshold=0`)
* **Execution Time:** ~0.2316s.
* **Overhead:** **~3069%**.
* **Total Events:** ~836,000+ events processed.
* **Impact:** Heaviest possible monitoring. Every single bytecode instruction triggers a Rust callback. Recommended only for micro-optimization of specific hot-spot functions, not whole-program tracing.

## Summary Table

| Mode | Threshold | Overhead | Best Use Case |
| :--- | :--- | :--- | :--- |
| **Adaptive** | 500 | **Minimal (<10%)** | Production profiling, long-running services. |
| **Adaptive** | 0 | **Moderate (500-800%)** | Accurate Perfetto timelines and loop counts. |
| **Precise** | 0 | **High (3000%+)** | Deep-dive instruction cycle analysis. |
