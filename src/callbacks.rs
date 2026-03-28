// callbacks.rs
use crate::model::TraceEvent;
use crate::state::{
    get_ts, init_tsc_calibration, is_perfetto_enabled, set_perfetto_enabled,
    DEINSTRUMENT_THRESHOLD, EVENT_QUEUE, FREE_QUEUE, IS_PRECISE, IS_RUNNING, WORKER_THREAD,
};
use crate::telemetry::telemetry_worker;
use crossbeam_queue::ArrayQueue;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyCode};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use std::thread;

const BATCH_SIZE: usize = 1024;

static DISABLE_OBJ: OnceLock<Py<PyAny>> = OnceLock::new();

thread_local! {
    static LOCAL_BATCH: RefCell<Vec<TraceEvent>> = RefCell::new(Vec::with_capacity(BATCH_SIZE));
    static SEEN_CODE_PTRS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    static HOT_OFFSETS: RefCell<HashMap<(usize, i32), u32>> = RefCell::new(HashMap::new());
}

#[inline(always)]
fn enqueue_event(event: TraceEvent) {
    LOCAL_BATCH.with(|batch_ref| {
        let mut batch = batch_ref.borrow_mut();
        batch.push(event);
        if batch.len() >= BATCH_SIZE {
            if let Some(queue) = EVENT_QUEUE.get() {
                let new_batch = FREE_QUEUE
                    .get()
                    .and_then(|q| q.pop())
                    .unwrap_or_else(|| Vec::with_capacity(BATCH_SIZE));
                let full_batch = std::mem::replace(&mut *batch, new_batch);
                let _ = queue.push(full_batch);
            } else {
                batch.clear();
            }
        }
    });
}

#[pyfunction]
#[pyo3(signature = (code, instruction_offset))]
pub fn instruction_callback(
    py: Python<'_>,
    code: &Bound<'_, PyAny>,
    instruction_offset: i32,
) -> Py<PyAny> {
    let code_ptr = code.as_ptr() as usize;
    enqueue_event(TraceEvent::Instruction {
        code_ptr,
        lasti: instruction_offset,
        tsc: crate::state::read_tsc(),
    });

    let threshold = DEINSTRUMENT_THRESHOLD.load(Ordering::Relaxed);
    let disable = if threshold > 0 {
        HOT_OFFSETS.with(|counts| {
            let mut map = counts.borrow_mut();
            let count = map.entry((code_ptr, instruction_offset)).or_insert(0);
            *count += 1;
            *count > threshold
        })
    } else {
        false
    };

    if disable {
        if let Some(disable_obj) = DISABLE_OBJ.get() {
            return disable_obj.clone_ref(py);
        }
    }
    py.None().into()
}

#[pyfunction]
#[pyo3(signature = (code, instruction_offset))]
pub fn py_start_callback(code: &Bound<'_, PyCode>, instruction_offset: i32) {
    let ts = if is_perfetto_enabled() { get_ts() } else { 0 };
    let code_ptr = code.as_ptr() as usize;

    let code_opt = SEEN_CODE_PTRS.with(|seen| {
        if seen.borrow_mut().insert(code_ptr) {
            Some(code.clone().unbind())
        } else {
            None
        }
    });

    enqueue_event(TraceEvent::PyStart {
        code: code_opt,
        code_ptr,
        lasti: instruction_offset,
        ts,
        tsc: crate::state::read_tsc(),
    });
}

#[pyfunction]
#[pyo3(signature = (code, instruction_offset, destination_offset))]
pub fn jump_callback(
    py: Python<'_>,
    code: &Bound<'_, PyAny>,
    instruction_offset: i32,
    destination_offset: i32,
) -> Py<PyAny> {
    let ts = if is_perfetto_enabled() { get_ts() } else { 0 };
    let code_ptr = code.as_ptr() as usize;
    enqueue_event(TraceEvent::Jump {
        code_ptr,
        from_lasti: instruction_offset,
        to_lasti: destination_offset,
        ts,
        tsc: crate::state::read_tsc(),
    });

    let threshold = DEINSTRUMENT_THRESHOLD.load(Ordering::Relaxed);
    let disable = if threshold > 0 {
        HOT_OFFSETS.with(|counts| {
            let mut map = counts.borrow_mut();
            let count = map.entry((code_ptr, instruction_offset)).or_insert(0);
            *count += 1;
            *count > threshold
        })
    } else {
        false
    };

    if disable {
        if let Some(disable_obj) = DISABLE_OBJ.get() {
            return disable_obj.clone_ref(py);
        }
    }
    py.None().into()
}

#[pyfunction]
#[pyo3(signature = (code, instruction_offset, retval))]
pub fn py_return_callback(
    code: &Bound<'_, PyAny>,
    instruction_offset: i32,
    retval: &Bound<'_, PyAny>,
) {
    let _ = instruction_offset;
    let _ = retval;
    let ts = if is_perfetto_enabled() { get_ts() } else { 0 };
    enqueue_event(TraceEvent::PyReturn {
        code_ptr: code.as_ptr() as usize,
        ts,
        tsc: crate::state::read_tsc(),
    });
}

#[pyfunction]
#[pyo3(signature = (mode="precise", perfetto=true, deinstrument_threshold=500))]
pub fn start_tracing(
    py: Python,
    mode: &str,
    perfetto: bool,
    deinstrument_threshold: u32,
) -> PyResult<()> {
    init_tsc_calibration();
    EVENT_QUEUE.get_or_init(|| ArrayQueue::new(10_000));
    FREE_QUEUE.get_or_init(|| {
        let q = ArrayQueue::new(10_000);
        for _ in 0..100 {
            let _ = q.push(Vec::with_capacity(BATCH_SIZE));
        }
        q
    });

    let is_precise = mode == "precise";
    IS_PRECISE.store(is_precise, Ordering::Relaxed);
    DEINSTRUMENT_THRESHOLD.store(deinstrument_threshold, Ordering::Relaxed);
    set_perfetto_enabled(perfetto);

    let mode_label = if is_precise { "precise" } else { "adaptive" };
    println!("[Ocular] ------------------------------------------------");
    println!("[Ocular] Starting Ocular tracing");
    println!("[Ocular] mode = {}", mode_label);
    println!("[Ocular] perfetto = {}", perfetto);
    println!("[Ocular] deinstrument_threshold = {}", deinstrument_threshold);
    println!("[Ocular] ------------------------------------------------");

    if !IS_RUNNING.swap(true, Ordering::Relaxed) {
        let handle = thread::spawn(telemetry_worker);
        if let Ok(mut thread_guard) = WORKER_THREAD.lock() {
            *thread_guard = Some(handle);
        }
    }

    let sys = py.import("sys")?;
    let sys_mon = sys.getattr("monitoring")?;

    DISABLE_OBJ.get_or_init(|| sys_mon.getattr("DISABLE").unwrap().unbind());

    let tool_id = sys_mon.getattr("DEBUGGER_ID")?;
    sys_mon.call_method0("restart_events")?;
    sys_mon.call_method1("use_tool_id", (tool_id.clone(), "ocular"))?;

    let events = sys_mon.getattr("events")?;
    let instruction_event = events.getattr("INSTRUCTION")?;
    let py_start_event = events.getattr("PY_START")?;
    let jump_event = events.getattr("JUMP")?;
    let branch_event = events.getattr("BRANCH")?;
    let py_return_event = events.getattr("PY_RETURN")?;

    let start_cb = pyo3::wrap_pyfunction!(py_start_callback)(py)?;
    sys_mon.call_method1(
        "register_callback",
        (tool_id.clone(), py_start_event.clone(), start_cb),
    )?;

    let jump_cb = pyo3::wrap_pyfunction!(jump_callback)(py)?;
    sys_mon.call_method1(
        "register_callback",
        (tool_id.clone(), jump_event.clone(), jump_cb.clone()),
    )?;
    sys_mon.call_method1(
        "register_callback",
        (tool_id.clone(), branch_event.clone(), jump_cb),
    )?;

    let return_cb = pyo3::wrap_pyfunction!(py_return_callback)(py)?;
    sys_mon.call_method1(
        "register_callback",
        (tool_id.clone(), py_return_event.clone(), return_cb),
    )?;

    let mut combined_events = py_start_event.extract::<i32>()?
        | jump_event.extract::<i32>()?
        | branch_event.extract::<i32>()?
        | py_return_event.extract::<i32>()?;

    if is_precise {
        let inst_cb = pyo3::wrap_pyfunction!(instruction_callback)(py)?;
        sys_mon.call_method1(
            "register_callback",
            (tool_id.clone(), instruction_event.clone(), inst_cb),
        )?;
        combined_events |= instruction_event.extract::<i32>()?;
    } else {
        sys_mon.call_method1(
            "register_callback",
            (tool_id.clone(), instruction_event.clone(), py.None()),
        )?;
    }

    sys_mon.call_method1("set_events", (tool_id, combined_events))?;

    Ok(())
}

#[pyfunction]
pub fn stop_tracing(py: Python) -> PyResult<()> {
    let sys = py.import("sys")?;
    let sys_mon = sys.getattr("monitoring")?;
    let tool_id = sys_mon.getattr("DEBUGGER_ID")?;

    let events = sys_mon.getattr("events")?;
    let instruction_event = events.getattr("INSTRUCTION")?;
    let py_start_event = events.getattr("PY_START")?;
    let jump_event = events.getattr("JUMP")?;
    let branch_event = events.getattr("BRANCH")?;
    let py_return_event = events.getattr("PY_RETURN")?;

    sys_mon.call_method1("set_events", (tool_id.clone(), 0))?;

    sys_mon.call_method1(
        "register_callback",
        (tool_id.clone(), instruction_event, py.None()),
    )?;
    sys_mon.call_method1(
        "register_callback",
        (tool_id.clone(), py_start_event, py.None()),
    )?;
    sys_mon.call_method1(
        "register_callback",
        (tool_id.clone(), jump_event, py.None()),
    )?;
    sys_mon.call_method1(
        "register_callback",
        (tool_id.clone(), branch_event, py.None()),
    )?;
    sys_mon.call_method1(
        "register_callback",
        (tool_id.clone(), py_return_event, py.None()),
    )?;

    sys_mon.call_method1("free_tool_id", (tool_id,))?;

    LOCAL_BATCH.with(|batch_ref| {
        let mut batch = batch_ref.borrow_mut();
        if !batch.is_empty() {
            if let Some(queue) = EVENT_QUEUE.get() {
                let new_batch = FREE_QUEUE
                    .get()
                    .and_then(|q| q.pop())
                    .unwrap_or_else(Vec::new);
                let final_batch = std::mem::replace(&mut *batch, new_batch);
                let _ = queue.push(final_batch);
            }
        }
    });

    SEEN_CODE_PTRS.with(|seen| {
        seen.borrow_mut().clear();
    });

    HOT_OFFSETS.with(|offsets| {
        offsets.borrow_mut().clear();
    });

    if IS_RUNNING.swap(false, Ordering::Relaxed) {
        if let Ok(mut thread_guard) = WORKER_THREAD.lock() {
            if let Some(handle) = thread_guard.take() {
                Python::detach(py, || {
                    let _ = handle.join();
                });
            }
        }
    }

    Ok(())
}
