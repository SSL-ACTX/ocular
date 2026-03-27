// model.rs
use pyo3::prelude::*;
use pyo3::types::PyCode;
use std::collections::HashMap;

/// Represents a singular event captured by the PEP 669 sensor.
#[derive(Debug)]
#[allow(dead_code)]
pub enum TraceEvent {
    PyStart {
        code: Option<Py<PyCode>>,
        code_ptr: usize,
        lasti: i32,
        ts: u64,
        tsc: u64,
    },
    Instruction {
        code_ptr: usize,
        lasti: i32,
        tsc: u64,
    },
    Jump {
        code_ptr: usize,
        from_lasti: i32,
        to_lasti: i32,
        ts: u64,
        tsc: u64,
    },
    PyReturn {
        code_ptr: usize,
        ts: u64,
        tsc: u64,
    },
}

/// Structure to hold cached offline metadata for a given PyCode object.
pub struct CodeMeta {
    pub name: String,
    pub code_obj: Py<PyCode>,
    pub base_opcodes: HashMap<i32, String>,
    pub valid_offsets: Vec<i32>, // Used to filter out inline caches
    pub filename: String,
    pub firstlineno: i32,
}

/// Aggregated statistics for a specific instruction sequence (Trace).
pub struct TraceStats {
    pub hits: u64,
    pub cycles: Vec<u64>,
}

#[cfg(feature = "perfetto")]
use serde::Serialize;

#[cfg(feature = "perfetto")]
#[derive(Serialize)]
pub struct PerfettoEvent {
    pub name: String,
    pub cat: String,
    pub ph: String,
    pub ts: u64,
    pub pid: u32,
    pub tid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<HashMap<String, String>>,
}
