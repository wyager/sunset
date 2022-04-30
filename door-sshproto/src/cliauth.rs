#[allow(unused_imports)]
use {
    crate::error::{Error, Result, TrapBug},
    log::{debug, error, info, log, trace, warn},
};

use heapless::Vec;

use crate::conn::RespPackets;
use crate::packets::Packet;
use crate::*;
use crate::sshnames::*;

pub(crate) struct CliAuth<'a> {
    username: &'a str,
    started: bool,
}

impl<'a> CliAuth<'a> {
    pub fn new() -> Self {
        CliAuth {
            started: false,
            username: "matt", // TODO. also this username length counts towards packet buffer size limit
        }
    }

    pub fn start(&mut self, resp: &mut RespPackets<'a>) -> Result<()> {
        if !self.started {
            self.started = true;
            resp.push(Packet::ServiceRequest(
                packets::ServiceRequest { name: SSH_SERVICE_USERAUTH })).trap()?;
            resp.push(Packet::UserauthRequest(
                packets::UserauthRequest {
                    username: self.username,
                    service: SSH_SERVICE_CONNECTION,
                    method: packets::AuthMethod::None,
                })).trap()?;
        }
        Ok(())
    }

    pub fn failure(&mut self, failure: &packets::UserauthFailure) -> Result<()> {
        Ok(())
    }

    pub fn success(&mut self, success: &packets::UserauthSuccess) -> Result<()> {
        Ok(())
    }
}
