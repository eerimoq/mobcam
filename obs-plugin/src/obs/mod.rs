mod clock;
pub mod data;
mod log;
pub mod media;
mod module;
pub mod properties;
mod source;
pub mod sys;
pub use clock::now_ns;
pub use data::{Data, DataArray, OwnedData};
pub use log::install_logger;
pub use media::{Audio, Frame};
pub use module::{Module, text};
pub use properties::{Properties, Property};
pub use source::{Source, register};

pub const API_VERSION: u32 =
    (sys::LIBOBS_API_MAJOR_VER << 24) | (sys::LIBOBS_API_MINOR_VER << 16) | sys::LIBOBS_API_PATCH_VER;
