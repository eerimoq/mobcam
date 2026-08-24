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

pub const API_VERSION: u32 =
    (sys::LIBOBS_API_MAJOR_VER << 24) | (sys::LIBOBS_API_MINOR_VER << 16) | sys::LIBOBS_API_PATCH_VER;
