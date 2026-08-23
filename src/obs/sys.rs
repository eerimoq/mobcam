//! The generated libobs FFI. Every call into it is made through the safe layer
//! next to it in `super`; what is used directly from here elsewhere in the
//! crate are the plain types and constants, which carry no unsafety.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code)]
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/obs.rs"));
