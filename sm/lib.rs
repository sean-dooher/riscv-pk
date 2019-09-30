#![no_std]

pub mod cpu;
pub mod sm;

#[allow(warnings)]
pub mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
