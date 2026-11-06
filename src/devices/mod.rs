//! Functional workstation devices. Host I/O and scheduling belong to the caller.
pub mod auxiliary;
pub mod esp;
pub mod ethernet;
pub mod ethernet_dma;
pub mod floppy;
pub mod interrupt;
pub mod io_control;
pub mod nvram;
pub mod reset;
pub mod scsi_dma;
pub mod serial;
pub mod timer;
