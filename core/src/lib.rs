#![no_std]

#[macro_use]
extern crate axlog;
extern crate alloc;

#[allow(
    dead_code,
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    missing_docs,
    clippy::all
)]
pub mod ctypes {
    include!(concat!(env!("OUT_DIR"), "/ctypes.rs"));
}

pub mod entry;
pub mod fd;
pub mod mm;
pub mod path;
pub mod task;
pub mod time;
