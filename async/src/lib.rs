#![allow(unused_imports)]

// avoid mysterious missing awaits
#![deny(unused_must_use)]

mod pty;
mod cmdline_client;
mod known_hosts;
mod agent;

#[cfg(unix)]
mod fdio;
#[cfg(unix)]
pub use fdio::{stdin, stdout, stderr_out};

pub use pty::{raw_pty, RawPtyGuard};

pub use cmdline_client::CmdlineClient;

pub use agent::AgentClient;

// for sshwire derive
use sunset::sshwire;
