mod callbacks;
mod model;
mod state;
mod telemetry;

use callbacks::{
    instruction_callback, jump_callback, py_return_callback, py_start_callback, start_tracing,
    stop_tracing,
};
use pyo3::prelude::*;
use pyo3::types::PyModule;

#[pymodule]
fn _ocular_core(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(start_tracing, m)?)?;
    m.add_function(wrap_pyfunction!(stop_tracing, m)?)?;
    m.add_function(wrap_pyfunction!(instruction_callback, m)?)?;
    m.add_function(wrap_pyfunction!(py_start_callback, m)?)?;
    m.add_function(wrap_pyfunction!(jump_callback, m)?)?;
    m.add_function(wrap_pyfunction!(py_return_callback, m)?)?;
    Ok(())
}
