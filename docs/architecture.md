# Ocular Architecture Reference

Ocular is designed to bridge CPython's PEP 669 (`sys.monitoring`) API with a high-performance Rust backend. The architecture prioritizes zero-allocation hot paths, hardware-level timing, and asynchronous telemetry processing to minimize the observer effect on the Python interpreter.

## Core Components

The system is divided into four primary subsystems: the FFI boundary, the lock-free event pipeline, the hardware timing module, and the asynchronous telemetry worker.

### 1. FFI Boundary & Sensor Hooks
The entry point from Python to Rust is managed via PyO3. Ocular registers specific C-level callbacks directly into CPython's monitoring system.

* Ocular registers callbacks for `PY_START`, `JUMP`, `BRANCH`, `PY_RETURN`, and optionally `INSTRUCTION` events.
* When a registered event fires in Python, CPython halts the eval loop and triggers the corresponding Rust `#[pyfunction]` (e.g., `instruction_callback`, `jump_callback`).
* To avoid hitting the Global Interpreter Lock (GIL) and atomic refcounting on every function call, `py_start_callback` caches `PyCode` object pointers in a thread-local `HashSet`.

### 2. Zero-Allocation Event Pipeline
To handle millions of events per second without garbage collection pauses or heap fragmentation, Ocular uses a batched, lock-free memory architecture.

* Events are initially pushed to a thread-local `LOCAL_BATCH` vector (`RefCell<Vec<TraceEvent>>`) to avoid synchronization overhead on every instruction.
* Once a batch reaches `BATCH_SIZE` (1024 events), it is pushed to a global, lock-free `crossbeam_queue::ArrayQueue` named `EVENT_QUEUE`.
* To prevent continuous allocation, Ocular utilizes a secondary `FREE_QUEUE`. When a batch is submitted to the telemetry worker, a pre-allocated empty batch is popped from the `FREE_QUEUE` to replace it.

### 3. Hardware Precision Timing
Standard OS system clock calls (`time.time()` or `Instant::now()`) are too slow for instruction-level profiling. Ocular bypasses the OS entirely.

* Execution cycles are measured using the CPU's Hardware Time Stamp Counter (TSC) via `read_tsc()`.
* This is implemented using inline assembly (`_rdtsc()` for x86_64 and `mrs cntvct_el0` for aarch64).
* At startup, `init_tsc_calibration()` measures the TSC frequency against standard time to accurately convert hardware cycles back to microseconds for the final output.

### 4. Dynamic De-instrumentation
To support adaptive profiling, Ocular can automatically detach itself from hot loops, allowing CPython to resume internal JIT/quickening optimizations.

* A thread-local `HOT_OFFSETS` map tracks how many times a specific `(code_ptr, instruction_offset)` pair has been executed.
* If the execution count exceeds the globally configured `DEINSTRUMENT_THRESHOLD`, the Rust callback returns CPython's special `DISABLE` object.
* Returning `DISABLE` instructs the Python VM to permanently strip the instrumentation hook from that specific bytecode offset, dropping Ocular's overhead for that loop to zero.

### 5. Asynchronous Telemetry Worker
Data aggregation and export are offloaded to a background OS thread to keep the Python eval loop moving as fast as possible.

* The `telemetry_worker` continuously pops full event batches from the `EVENT_QUEUE`.
* It dynamically imports Python's `dis` module via the GIL to reverse-engineer opcodes (e.g., mapping a raw offset to `BINARY_SUBSCR_LIST_INT`) and caches this in a `CodeMeta` struct.
* The worker reconstructs the trace sequences, calculates average hardware cycles per micro-operation, and ranks the hottest loops (`TraceStats`).
* If Perfetto is enabled, the worker formats the function boundaries into `PerfettoEvent` structs and flushes them to `ocular_trace.json` upon exit.
