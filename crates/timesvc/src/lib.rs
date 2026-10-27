//! Time service (task #212): [`ntp`] SNTPv4 client logic, [`tz`] POSIX-TZ rules and local
//! time conversion, [`calendar`] civil date math. Pure logic; the service owns sockets and RTC.
//! Privacy: no default servers are baked in; the user/admin configures them.
#![no_std]

extern crate alloc;

pub mod calendar;
pub mod ntp;
pub mod tz;
