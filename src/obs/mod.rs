//! The parts of libobs the plugin uses, wrapped just enough to be safe.

pub mod sys;

mod log;

pub use log::{log, Level};

/// The libobs version the plugin was built against, which is what
/// `obs_module_ver` has to report. Taken from the headers CMake resolved, so an
/// OBS upgrade cannot leave a stale number behind.
pub const API_VERSION: u32 =
    (sys::LIBOBS_API_MAJOR_VER << 24) | (sys::LIBOBS_API_MINOR_VER << 16) | sys::LIBOBS_API_PATCH_VER;
