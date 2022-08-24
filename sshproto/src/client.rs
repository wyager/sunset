#[allow(unused_imports)]
use {
    crate::error::{Error, Result, TrapBug},
    log::{debug, error, info, log, trace, warn},
};

use snafu::prelude::*;

use crate::{*, packets::ChannelOpen};
use packets::{Packet, PubKey, ParseContext};
use traffic::TrafSend;
use sshnames::*;
use cliauth::CliAuth;
use sign::SignKey;
use behaviour::CliBehaviour;
use heapless::String;

pub(crate) struct Client {
    pub auth: CliAuth,
}

impl Client {
    pub fn new() -> Self {
        Client {
            auth: CliAuth::new(),
        }
    }

    // pub fn check_hostkey(hostkey: )

    pub(crate) fn auth_success(&mut self,
        parse_ctx: &mut ParseContext,
        s: &mut TrafSend,
        b: &mut dyn CliBehaviour) -> Result<()> {

        parse_ctx.cli_auth_type = None;
        s.send(packets::ServiceRequest { name: SSH_SERVICE_CONNECTION })?;
        self.auth.success(b)
    }

    pub(crate) fn banner(&mut self, banner: &packets::UserauthBanner<'_>, b: &mut dyn CliBehaviour) {
        b.show_banner(banner.message, banner.lang)
    }
}
