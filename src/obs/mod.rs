//! The parts of libobs the plugin uses, wrapped just enough to be safe.

pub mod sys;

mod clock;
pub mod data;
mod log;
pub mod media;
mod module;
pub mod properties;
mod source;

pub use clock::now_ns;
pub use data::{Data, OwnedData};
pub use log::{log, Level};
pub use media::{Audio, Frame};
pub use module::{text, Module};
pub use properties::{Properties, Property};
pub use source::{register, Source};

/// The libobs version the plugin was built against, which is what
/// `obs_module_ver` has to report. Taken from the headers the plugin is built
/// against, so an OBS upgrade cannot leave a stale number behind.
pub const API_VERSION: u32 =
    (sys::LIBOBS_API_MAJOR_VER << 24) | (sys::LIBOBS_API_MINOR_VER << 16) | sys::LIBOBS_API_PATCH_VER;
