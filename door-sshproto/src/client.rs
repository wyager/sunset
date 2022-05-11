#[allow(unused_imports)]
use {
    crate::error::{Error, Result, TrapBug},
    log::{debug, error, info, log, trace, warn},
};

use snafu::prelude::*;

use crate::{*, packets::ChannelOpen};
use crate::packets::{Packet, PubKey};
use crate::sshnames::*;
use crate::cliauth::CliAuth;
use crate::conn::RespPackets;
use crate::sign::SignKey;
use heapless::String;

pub struct Client<'a> {
    pub(crate) auth: CliAuth,
    pub(crate) hooks: RefHooks<'a>,
}

impl<'a> Client<'a> {
    pub fn new(hooks: RefHooks<'a>) -> Result<Self> {
        Ok(Client {
            auth: CliAuth::new(),
            hooks,
        })
    }

    // pub fn check_hostkey(hostkey: )

    pub fn auth_success(&mut self, resp: &mut RespPackets) -> Result<()> {
        resp.push(Packet::ServiceRequest(
            packets::ServiceRequest { name: SSH_SERVICE_CONNECTION } )).trap()?;
        let h = self.auth.success(self.hooks)?;
        if h.open_session {
            // TODDO
        //     resp.push(packets::Packet::ChannelOpen(ChannelOpen {
        //         number: 
        //     }
        }
        Ok(())
    }

    pub fn banner(&mut self, banner: &packets::UserauthBanner) {
        self.hooks.show_banner(banner.message, banner.lang);
    }
}

// A bit of a mouthful.
pub(crate) type RefHooks<'a> = &'a mut dyn ClientHooks<'a>;

/// A stack-allocated string to store responses for usernames or passwords.
// 100 bytes is an arbitrary size.
pub type ResponseString = heapless::String<100>;

#[derive(Debug,Snafu)]
pub enum HookError {
    Fail,
    #[doc(hidden)]
    Unimplemented,
}

// TODO: probably want a special Result here. They probably all want
// Result, it can return an error or other options like Disconnect?
pub type HookResult<T> = core::result::Result<T, HookError>;

pub struct ClientHandle {
    open_session: bool,
    pty: bool,
}

impl ClientHandle {
    pub(crate) fn new() -> Self {
        Self {
            open_session: false,
            pty: false,
        }
    }

    pub fn open_session(&mut self, pty: bool) -> Result<()> {
        if self.open_session {
            return Err(Error::Custom { msg: "Only one session can be opened per callback" })
        }
        self.open_session = true;
        self.pty = pty;
        Ok(())
    }

}

///
/// # Examples
///
/// ` ``
/// fn auth_password(&mut self) -> Option<ResponseString> {
/// // TODO
/// }
/// ` ``
pub trait ClientHooks<'a> {
    /// Provide the username to use for authentication. Will only be called once
    /// per session.
    /// If the username needs to change a new connection should be made
    /// – servers often have limits on authentication attempts.
    ///
    fn username(&mut self, username: &mut ResponseString) -> HookResult<()>;

    /// Whether to accept a hostkey for the server. The implementation
    /// should compare the key with the key expected for the hostname used.
    fn valid_hostkey(&mut self, key: &PubKey) -> HookResult<bool>;

    /// Get a password to use for authentication returning `Ok(true)`.
    /// Return `Ok(false)` to skip password authentication
    // TODO: having the hostname and username is useful to build a prompt?
    // or we could provide a full prompt as Args
    #[allow(unused)]
    fn auth_password(&mut self, pwbuf: &mut ResponseString) -> HookResult<bool> {
        Ok(false)
    }

    /// Get the next private key to authenticate with. Will not be called
    /// again once returning `HookError::Skip`
    /// The default implementation returns `HookError::Skip`
    fn next_authkey(&mut self) -> HookResult<Option<SignKey>> {
        Ok(None)
    }

    /// Called after authentication has succeeded
    fn authenticated(&mut self, client: &mut ClientHandle) -> HookResult<()>;

    /// Show a banner sent from a server. Arguments are provided
    /// by the server so could be hazardous, they should be escaped with
    /// [`banner.escape_default()`](core::str::escape_default) or similar.
    /// Language may be empty, is provided by the server.
    #[allow(unused)]
    fn show_banner(&self, banner: &str, language: &str) -> HookResult<()> {
        info!("Got banner:\n{}", banner.escape_default());
        Ok(())
    }
    // TODO: postauth channel callbacks
}
