// telemetry.rs
use crate::model::{CodeMeta, PerfettoEvent, TraceEvent, TraceStats};
use crate::state::{EVENT_QUEUE, FREE_QUEUE, IS_PRECISE, IS_RUNNING, DEINSTRUMENT_THRESHOLD, is_perfetto_enabled, get_exclude_patterns, get_include_patterns};
use pyo3::prelude::*;
use pyo3::types::PyDict;

const HOT_TRACE_EXPORT: bool = false;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

pub fn telemetry_worker() {
    let queue = EVENT_QUEUE
        .get()
        .expect("EVENT_QUEUE must be initialized before worker starts");
    let mut processed_events: u64 = 0;

    let mut code_registry: HashMap<usize, CodeMeta> = HashMap::new();
    let mut code_hot_hits: HashMap<usize, u64> = HashMap::new();
    let mut call_stack: Vec<(usize, i32, u64)> = Vec::with_capacity(64);

    let mut current_trace: Vec<(usize, i32, u64)> = Vec::with_capacity(256);
    let mut current_trace_cycles: u64 = 0;

    let mut hot_traces: HashMap<Vec<(usize, i32)>, TraceStats> = HashMap::new();
    let mut perfetto_events: Vec<PerfettoEvent> = Vec::with_capacity(10_000);

    while IS_RUNNING.load(std::sync::atomic::Ordering::Relaxed) || !queue.is_empty() {
        if let Some(mut batch) = queue.pop() {
            for event in batch.drain(..) {
                processed_events += 1;

                match event {
                    TraceEvent::PyStart {
                        code,
                        code_ptr,
                        ts,
                        lasti,
                        tsc,
                    } => {
                        #[cfg(not(feature = "perfetto"))]
                        let _ = ts;

                        if let Some(code_obj) = code {
                            if !code_registry.contains_key(&code_ptr) {
                                Python::attach(|py| {
                                    let bound_code = code_obj.bind(py);

                                    let func_name = bound_code
                                        .getattr("co_name")
                                        .and_then(|n| n.extract::<String>())
                                        .unwrap_or_else(|_| "unknown".to_string());

                                    let mut base_opcodes = HashMap::new();
                                    let mut valid_offsets = Vec::new();

                                    if let Ok(dis) = py.import("dis") {
                                        let kwargs = PyDict::new(py);
                                        let _ = kwargs.set_item("adaptive", false);

                                        if let Ok(instructions) = dis.call_method(
                                            "get_instructions",
                                            (bound_code,),
                                            Some(&kwargs),
                                        ) {
                                            if let Ok(iter) = instructions.try_iter() {
                                                for inst in iter {
                                                    if let Ok(inst) = inst {
                                                        let offset = inst
                                                            .getattr("offset")
                                                            .ok()
                                                            .and_then(|o| o.extract::<i32>().ok());
                                                        let opname =
                                                            inst.getattr("opname").ok().and_then(
                                                                |o| o.extract::<String>().ok(),
                                                            );
                                                        let arg = inst
                                                            .getattr("arg")
                                                            .ok()
                                                            .and_then(|o| o.extract::<i32>().ok());
                                                        let argrepr =
                                                            inst.getattr("argrepr").ok().and_then(
                                                                |o| o.extract::<String>().ok(),
                                                            );
                                                        let starts_line = inst
                                                            .getattr("starts_line")
                                                            .ok()
                                                            .and_then(|o| o.extract::<i32>().ok());
                                                        let is_jump_target = inst
                                                            .getattr("is_jump_target")
                                                            .ok()
                                                            .and_then(|o| o.extract::<bool>().ok())
                                                            .unwrap_or(false);

                                                        if let (Some(off), Some(name)) =
                                                            (offset, opname)
                                                        {
                                                            base_opcodes.insert(
                                                                off,
                                                                crate::model::InstMeta {
                                                                    opname: name,
                                                                    arg,
                                                                    argrepr,
                                                                    starts_line,
                                                                    is_jump_target,
                                                                },
                                                            );
                                                            valid_offsets.push(off);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    valid_offsets.sort();

                                    let filename = bound_code
                                        .getattr("co_filename")
                                        .and_then(|f| f.extract::<String>())
                                        .unwrap_or_else(|_| "<unknown>".to_string());
                                    let firstlineno = bound_code
                                        .getattr("co_firstlineno")
                                        .and_then(|f| f.extract::<i32>())
                                        .unwrap_or(-1);

                                    code_registry.insert(
                                        code_ptr,
                                        CodeMeta {
                                            name: func_name,
                                            code_obj: code_obj.clone_ref(py),
                                            base_opcodes,
                                            valid_offsets,
                                            filename: filename.clone(),
                                            firstlineno,
                                        },
                                    );
                                });
                            }
                        }


                        call_stack.push((code_ptr, lasti, tsc));

                        if is_perfetto_enabled() {
                            let name = code_registry
                                .get(&code_ptr)
                                .map(|m| m.name.clone())
                                .unwrap_or_else(|| "unknown".to_string());

                            let mut event_args = HashMap::new();
                            event_args.insert("code_ptr".to_string(), format!("{}", code_ptr));
                            perfetto_events.push(PerfettoEvent {
                                name,
                                cat: "function".to_string(),
                                ph: "B".to_string(),
                                ts,
                                pid: 1,
                                tid: 1,
                                args: Some(event_args),
                            });
                        }
                    }
                    TraceEvent::Instruction {
                        code_ptr,
                        lasti,
                        tsc,
                    } => {
                        current_trace.push((code_ptr, lasti, tsc));
                    }
                    TraceEvent::Jump {
                        code_ptr,
                        from_lasti,
                        to_lasti,
                        ts,
                        tsc,
                    } => {
                        #[cfg(not(feature = "perfetto"))]
                        let _ = ts;
                        if let Some(top) = call_stack.last_mut() {
                            if top.0 == code_ptr {
                                let start_pc = top.1;
                                let last_tsc = top.2;
                                let is_precise =
                                    IS_PRECISE.load(std::sync::atomic::Ordering::Relaxed);

                                if !is_precise {
                                    if let Some(meta) = code_registry.get(&code_ptr) {
                                        for &offset in &meta.valid_offsets {
                                            if offset >= start_pc && offset <= from_lasti {
                                                current_trace.push((code_ptr, offset, 0));
                                            }
                                        }
                                    }
                                }

                                let block_cycles = tsc.saturating_sub(last_tsc);
                                current_trace_cycles += block_cycles;

                                top.1 = to_lasti;
                                top.2 = tsc;

                                if to_lasti < from_lasti && !current_trace.is_empty() {
                                    let trace_key: Vec<(usize, i32)> =
                                        current_trace.iter().map(|&(c, l, _)| (c, l)).collect();
                                    let len = current_trace.len();

                                    let stats =
                                        hot_traces.entry(trace_key).or_insert_with(|| TraceStats {
                                            hits: 0,
                                            cycles: vec![0; len],
                                        });
                                    stats.hits += 1;

                                    let mut total_trace_cycles: u64 = 0;

                                    if !is_precise {
                                        let avg = if len > 0 {
                                            current_trace_cycles / len as u64
                                        } else {
                                            0
                                        };
                                        for i in 0..len {
                                            stats.cycles[i] += avg;
                                            if is_perfetto_enabled() {
                                                total_trace_cycles += avg;
                                            }
                                        }
                                    } else {
                                        for i in 0..len {
                                            let current_tsc = current_trace[i].2;
                                            let next_tsc = if i + 1 < len {
                                                current_trace[i + 1].2
                                            } else {
                                                tsc
                                            };
                                            let delta = next_tsc.saturating_sub(current_tsc);
                                            stats.cycles[i] += delta;
                                            if is_perfetto_enabled() {
                                                total_trace_cycles += delta;
                                            }
                                        }
                                    }

                                    *code_hot_hits.entry(code_ptr).or_insert(0) += 1;

                                    if is_perfetto_enabled() {
                                        let name = code_registry
                                            .get(&code_ptr)
                                            .map(|m| m.name.clone())
                                            .unwrap_or_else(|| "unknown".to_string());
                                        let mut args = HashMap::new();
                                        args.insert("uOps".to_string(), len.to_string());
                                        args.insert(
                                            "Hardware Cycles".to_string(),
                                            total_trace_cycles.to_string(),
                                        );
                                        args.insert(
                                            "code_ptr".to_string(),
                                            code_ptr.to_string(),
                                        );

                                        perfetto_events.push(PerfettoEvent {
                                            name: format!("{} (Loop)", name),
                                            cat: "loop".to_string(),
                                            ph: "i".to_string(),
                                            ts,
                                            pid: 1,
                                            tid: 1,
                                            args: Some(args),
                                        });
                                    }

                                    current_trace.clear();
                                    current_trace_cycles = 0;
                                }
                            }
                        }
                    }
                    TraceEvent::PyReturn {
                        code_ptr,
                        ts,
                        tsc: _,
                    } => {
                        #[cfg(not(feature = "perfetto"))]
                        let _ = ts;

                        if let Some((top_code_ptr, _, _)) = call_stack.last() {
                            if *top_code_ptr == code_ptr {
                                call_stack.pop();
                            } else {
                                while let Some(top) = call_stack.last() {
                                    if top.0 == code_ptr {
                                        call_stack.pop();
                                        break;
                                    }
                                    call_stack.pop();
                                }
                            }
                        }

                        current_trace.clear();
                        current_trace_cycles = 0;

                        if is_perfetto_enabled() {
                            let name = code_registry
                                .get(&code_ptr)
                                .map(|m| m.name.clone())
                                .unwrap_or_else(|| "unknown".to_string());

                            let mut event_args = HashMap::new();
                            event_args.insert("code_ptr".to_string(), code_ptr.to_string());
                            perfetto_events.push(PerfettoEvent {
                                name,
                                cat: "function".to_string(),
                                ph: "E".to_string(),
                                ts,
                                pid: 1,
                                tid: 1,
                                args: Some(event_args),
                            });
                        }
                    }
                }
            }

            if let Some(free_q) = FREE_QUEUE.get() {
                let _ = free_q.push(batch);
            }
        } else if IS_RUNNING.load(std::sync::atomic::Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(1));
        }
    }

    println!(
        "[Ocular] Telemetry worker gracefully exited. Processed {} events.",
        processed_events
    );

    if is_perfetto_enabled() {
        #[cfg(feature = "perfetto")]
        {
            if let Ok(file) = File::create("ocular_trace.json") {
                let writer = BufWriter::new(file);
                if serde_json::to_writer(writer, &perfetto_events).is_ok() {
                    println!("[Ocular] 🗄️ Perfetto timeline exported to 'ocular_trace.json'");
                }
            }
        }

        #[cfg(not(feature = "perfetto"))]
        {
            println!("[Ocular] Perfetto tracing enabled at runtime, but crate built without perfetto feature; no JSON export.");
        }
    }

    let mode_label = if IS_PRECISE.load(Ordering::Relaxed) {
        "precise"
    } else {
        "adaptive"
    };
    let perfetto_active = is_perfetto_enabled();
    let deinstrument_threshold = DEINSTRUMENT_THRESHOLD.load(Ordering::Relaxed);
    let exclude_patterns = get_exclude_patterns();
    let include_patterns = get_include_patterns();

    println!("[Ocular] =================================================");
    println!("[Ocular] 🧭 Ocular Telemetry UI");
    println!("[Ocular] mode                 = {}", mode_label);
    println!("[Ocular] perfetto             = {}", perfetto_active);
    println!("[Ocular] deinstrument_threshold = {}", deinstrument_threshold);
    if !exclude_patterns.is_empty() {
        println!("[Ocular] exclude patterns      = {:?}", exclude_patterns);
    }
    if !include_patterns.is_empty() {
        println!("[Ocular] include_only patterns = {:?}", include_patterns);
    }
    println!("[Ocular] active threads       = {:?}", thread::available_parallelism().ok());
    println!("[Ocular] =================================================");

    if let Some((top_trace, stats)) = hot_traces.into_iter().max_by_key(|entry| entry.1.hits) {
        let user_codes: Vec<_> = code_hot_hits
            .iter()
            .filter_map(|(&c_ptr, &hits)| {
                code_registry.get(&c_ptr).and_then(|meta| {
                    let is_std = meta.filename.starts_with("/usr")
                        || meta.filename.contains("<frozen importlib")
                        || meta.filename.contains("<string>");
                    if !is_std {
                        Some((c_ptr, hits, meta.name.as_str(), meta.filename.as_str()))
                    } else {
                        None
                    }
                })
            })
            .collect();

        let selected_code = user_codes
            .iter()
            .max_by_key(|(_, hits, _, _)| *hits)
            .map(|(c_ptr, _, _, _)| *c_ptr)
            .or_else(|| {
                code_hot_hits
                    .iter()
                    .max_by_key(|entry| entry.1)
                    .map(|(cp, _)| *cp)
            });

        if let Some(c_ptr) = selected_code {
            let func_name = code_registry
                .get(&c_ptr)
                .map(|m| m.name.as_str())
                .unwrap_or("unknown");
            println!("[Ocular] Preferred hot code: 0x{:x} ({})", c_ptr, func_name);
        }
        println!("[Ocular] ------------------------------------------------");
        println!("[Ocular] 🎯 Top Hot Trace Detected:");
        println!("[Ocular] Hot trace length : {} uOps", top_trace.len());

        if let Some((&c_ptr, &hits)) = code_hot_hits.iter().max_by_key(|entry| entry.1) {
            let func_name = code_registry
                .get(&c_ptr)
                .map(|m| m.name.as_str())
                .unwrap_or("unknown");
            println!(
                "[Ocular] Active hot code: 0x{:x} ({}) hits: {}",
                c_ptr, func_name, hits
            );
        }

        println!("[Ocular] Hits:   {}", stats.hits);
        println!("[Ocular] Trace Disassembly with Hardware CPU Cycles (Base -> Specialized):");
        println!("[Ocular]   #   code_ptr          line lbl instruction                             arg  repr           cycles");
        println!("[Ocular] ------------------------------------------------");

        let mut trace_dump_lines = Vec::new();
        let is_precise = IS_PRECISE.load(std::sync::atomic::Ordering::Relaxed);

        if let Some((&c_ptr, &code_hits)) = code_hot_hits.iter().max_by_key(|entry| entry.1) {
            let code_info = code_registry
                .get(&c_ptr)
                .map(|m| format!("{} ({}:{})", m.name, m.filename, m.firstlineno))
                .unwrap_or_else(|| "unknown".to_string());
            trace_dump_lines.push(format!("Ocular Hot Trace Dump"));
            trace_dump_lines.push(format!(
                "Active hot code: 0x{:x} -> {} hits: {}",
                c_ptr, code_info, code_hits
            ));
        } else {
            trace_dump_lines.push(format!("Ocular Hot Trace Dump"));
        }

        trace_dump_lines.push(format!("Length: {} uOps", top_trace.len()));
        trace_dump_lines.push(format!("Hits:   {}", stats.hits));
        trace_dump_lines.push(format!("------------------------------------------------"));

        let mut offset_to_label: HashMap<i32, String> = HashMap::new();
        let mut label_counter = 1;

        // Pre-compute jump labels for the UI
        if let Some(&c_ptr) = selected_code.as_ref() {
            if let Some(meta) = code_registry.get(&c_ptr) {
                for &offset in &meta.valid_offsets {
                    if let Some(inst) = meta.base_opcodes.get(&offset) {
                        if inst.is_jump_target {
                            offset_to_label.insert(offset, format!("L{}", label_counter));
                            label_counter += 1;
                        }
                    }
                }
            }
        }

        let mut last_printed_line = -1;

        Python::attach(|py| {
            let mut disassembly_cache: HashMap<usize, HashMap<i32, String>> = HashMap::new();
            let dis_module = py.import("dis").ok();

            for (idx, (c_ptr, lasti)) in top_trace.into_iter().enumerate() {
                let mut opcode_base = "UNKNOWN".to_string();
                let mut opcode_quickened = "UNKNOWN".to_string();
                let mut arg_val = None;
                let mut arg_repr = None;
                let mut starts_line = None;

                if let Some(meta) = code_registry.get(&c_ptr) {
                    if let Some(inst_meta) = meta.base_opcodes.get(&lasti) {
                        opcode_base = inst_meta.opname.clone();
                        arg_val = inst_meta.arg;
                        arg_repr = inst_meta.argrepr.clone();
                        starts_line = inst_meta.starts_line;
                    }

                    let inst_map = disassembly_cache.entry(c_ptr).or_insert_with(|| {
                        let mut map = HashMap::new();
                        if let Some(dis) = &dis_module {
                            let kwargs = PyDict::new(py);
                            let _ = kwargs.set_item("adaptive", true);

                            // Python 3.14+ safety catch for instrumented disassemblies
                            let mut dis_result = dis.call_method(
                                "get_instructions",
                                (meta.code_obj.bind(py),),
                                Some(&kwargs),
                            );

                            if dis_result.is_err() {
                                let fallback_kwargs = PyDict::new(py);
                                let _ = fallback_kwargs.set_item("adaptive", false);
                                dis_result = dis.call_method(
                                    "get_instructions",
                                    (meta.code_obj.bind(py),),
                                    Some(&fallback_kwargs),
                                );
                            }

                            if let Ok(instructions) = dis_result {
                                if let Ok(iter) = instructions.try_iter() {
                                    for inst in iter {
                                        if let Ok(inst) = inst {
                                            let offset = inst
                                                .getattr("offset")
                                                .ok()
                                                .and_then(|o| o.extract::<i32>().ok());
                                            let opname = inst
                                                .getattr("opname")
                                                .ok()
                                                .and_then(|o| o.extract::<String>().ok());
                                            if let (Some(off), Some(name)) = (offset, opname) {
                                                map.insert(off, name);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        map
                    });

                    opcode_quickened = inst_map
                        .get(&lasti)
                        .cloned()
                        .map(|n| n.replace("INSTRUMENTED_", ""))
                        .unwrap_or_else(|| opcode_base.clone());
                }

                let avg_cycles = stats.cycles[idx] / stats.hits;
                let transition = if opcode_base == opcode_quickened {
                    opcode_base
                } else {
                    format!("{} -> {}", opcode_base, opcode_quickened)
                };

                let line_prefix = if let Some(line) = starts_line {
                    if line != last_printed_line {
                        last_printed_line = line;
                        format!("{:>3}", line)
                    } else {
                        "   ".to_string()
                    }
                } else {
                    "   ".to_string()
                };

                let label_prefix = if let Some(label) = offset_to_label.get(&lasti) {
                    format!("{:>3}:", label)
                } else {
                    "    ".to_string()
                };

                let arg_str = if let Some(arg) = arg_val {
                    format!("{:<3}", arg)
                } else {
                    "   ".to_string()
                };

                // Transform "to 18" -> "to L2" for UI matching
                let argrepr_str = if let Some(repr) = arg_repr {
                    if repr.starts_with("to ") {
                        if let Ok(target_offset) = repr[3..].parse::<i32>() {
                            if let Some(lbl) = offset_to_label.get(&target_offset) {
                                format!("(to {})", lbl)
                            } else {
                                format!("({})", repr)
                            }
                        } else {
                            format!("({})", repr)
                        }
                    } else {
                        format!("({})", repr)
                    }
                } else {
                    "".to_string()
                };

                let opcode_line = format!(
                    "{:>3} 0x{:016x}  {} {}  {:<40} {:<4} {:<15}",
                    idx,
                    c_ptr,
                    line_prefix,
                    label_prefix,
                    transition,
                    arg_str,
                    argrepr_str
                );

                let line = if !is_precise {
                    format!(
                        "[Ocular] {} | ~{} cycles (avg block latency)",
                        opcode_line, avg_cycles
                    )
                } else {
                    format!("[Ocular] {} | ~{} cycles", opcode_line, avg_cycles)
                };

                println!("{}", line);
                trace_dump_lines.push(line);
            }
        });

        println!("[Ocular] ------------------------------------------------");

        if HOT_TRACE_EXPORT {
            if let Ok(file) = File::create("ocular_hot_trace.txt") {
                let mut writer = BufWriter::new(file);
                for line in trace_dump_lines {
                    let _ = writeln!(writer, "{}", line);
                }
                println!("[Ocular] 📄 Hot trace disassembly exported to 'ocular_hot_trace.txt'");
            }
        }
    } else {
        println!("[Ocular] No hot traces (loops) detected.");
    }
}
