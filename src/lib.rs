//! py-spy: a sampling profiler for python programs
//!
//! This crate lets you use py-spy as a rust library, and gather stack traces from
//! your python process programmatically.
//!
//! # Example:
//!
//! ```rust,no_run
//! fn print_python_stacks(pid: py_spy::Pid) -> Result<(), failure::Error> {
//!     // Create a new PythonSpy object with the default config options
//!     let config = py_spy::Config::default();
//!     let mut process = py_spy::PythonSpy::new(pid, &config)?;
//!
//!     // get stack traces for each thread in the process
//!     let traces = process.get_stack_traces()?;
//!
//!     // Print out the python stack for each thread
//!     for trace in traces {
//!         for frame in &trace.frames {
//!         }
//!     }
//!     Ok(())
//! }
//! ```

#[macro_use]
extern crate clap;
#[macro_use]
extern crate failure;
extern crate goblin;
#[macro_use]
extern crate lazy_static;
extern crate libc;
#[macro_use]
extern crate log;
extern crate cpp_demangle;
#[cfg(unwind)]
extern crate lru;
extern crate memmap;
extern crate proc_maps;
extern crate rand;
extern crate rand_distr;
extern crate regex;
extern crate remoteprocess;
#[cfg(windows)]
extern crate winapi;

pub mod binary_parser;
pub mod config;
#[cfg(unwind)]
mod cython;
#[cfg(unwind)]
mod native_stack_trace;
mod python_bindings;
mod python_data_access;
mod python_interpreters;
mod python_spy;
mod python_threading;
pub mod sampler;
mod stack_trace;
pub mod timer;
mod utils;
mod version;

pub use config::Config;
pub use python_spy::PythonSpy;
pub use remoteprocess::Pid;
pub use sampler::Sampler;
pub use stack_trace::Frame;
pub use stack_trace::StackTrace;

use crate::config::LockingStrategy;
use std::collections::HashMap;
use std::slice;
use std::sync::Mutex;

use rand::thread_rng;
use rand::seq::SliceRandom;

lazy_static! {
    static ref HASHMAP: Mutex<HashMap<Pid, Sampler>> = {
        let h = HashMap::new();
        Mutex::new(h)
    };
}

fn copy_error(err_ptr: *mut u8, err_len: i32, err_str: String) -> i32 {
    let slice = err_str.as_bytes();
    let l = slice.len();
    if l as i32 > err_len {
        return copy_error(err_ptr, err_len, "buffer is too small".to_string());
    }
    let target = unsafe { slice::from_raw_parts_mut(err_ptr, l as usize) };
    target.clone_from_slice(slice);
    -(l as i32)
}

#[no_mangle]
pub extern "C" fn pyspy_init(pid: Pid, blocking: i32, err_ptr: *mut u8, err_len: i32) -> i32 {
    let mut config = config::Config::default();
    if blocking == 0 {
        config.blocking = LockingStrategy::NonBlocking;
    }
    match Sampler::new(pid, &config) {
        Ok(sampler) => {
            let mut map = HASHMAP.lock().unwrap(); // get()
            map.insert(pid, sampler);
            1
        }
        Err(err) => copy_error(err_ptr, err_len, err.to_string()),
    }
}

#[no_mangle]
pub extern "C" fn pyspy_cleanup(pid: Pid, err_ptr: *mut u8, err_len: i32) -> i32 {
    let mut map = HASHMAP.lock().unwrap(); // get()
    map.remove(&pid);
    1
}

#[no_mangle]
pub extern "C" fn pyspy_snapshot(
    pid: Pid,
    ptr: *mut u8,
    len: i32,
    err_ptr: *mut u8,
    err_len: i32,
) -> i32 {
    let mut map = HASHMAP.lock().unwrap(); // get()
    match map.get_mut(&pid) {
        Some(sampler) => {
            for sample in sampler {
                let mut string_list = vec![];
                let mut traces: Vec<StackTrace> = sample.traces;
                traces.shuffle(&mut thread_rng());

                for thread in traces.iter() {
                    if !thread.active {
                        continue;
                    }
                    for frame in &thread.frames {
                        let filename = match &frame.short_filename {
                            Some(f) => &f,
                            None => &frame.filename,
                        };
                        if frame.line != 0 {
                            string_list
                                .insert(0, format!("{}:{} - {}", filename, frame.line, frame.name));
                        } else {
                            string_list.insert(0, format!("{} - {}", filename, frame.name));
                        }
                    }
                    break;
                }
                let joined = string_list.join(";");
                let joined_slice = joined.as_bytes();
                let l = joined_slice.len();

                if len < (l as i32) {
                    // println!("buffer is too small");
                    // io::stdout().flush().unwrap();
                    return copy_error(err_ptr, err_len, "buffer is too small".to_string());
                } else {
                    let slice = unsafe { slice::from_raw_parts_mut(ptr, l as usize) };
                    slice.clone_from_slice(joined_slice);
                    return l as i32;
                }
            }

            return 0;
        }
        None => copy_error(
            err_ptr,
            err_len,
            "could not find spy for this pid".to_string(),
        ),
    }
}
