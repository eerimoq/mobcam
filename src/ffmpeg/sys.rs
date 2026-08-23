//! The generated FFmpeg FFI. Nothing outside this module should use it
//! directly; the safe layer lives next to it in `super`.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code)]
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/ffmpeg.rs"));
