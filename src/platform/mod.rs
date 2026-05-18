#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{age_seconds, file_id};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{age_seconds, file_id};
