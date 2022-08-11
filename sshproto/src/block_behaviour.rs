// see comments in behaviour.rs
#![cfg(not(feature = "std"))]

#[allow(unused_imports)]
use {
    crate::error::{Error, Result, TrapBug},
    log::{debug, error, info, log, trace, warn},
};

use crate::{conn::RespPackets, *};
use behaviour::*;

pub(crate) enum BlockCliServ<'a> {
    Client(&'a mut dyn BlockCliBehaviour),
    Server(&'a mut dyn BlockServBehaviour),
}

impl BlockCliServ<'_>
{
    pub fn client(&mut self) -> Result<CliBehaviour> {
        let c = match self {
            Self::Client(c) => c,
            _ => Error::bug_msg("Not client")?,
        };
        let c = CliBehaviour {
            inner: *c,
        };
        Ok(c)
    }

    pub fn server(&mut self) -> Result<ServBehaviour> {
        let c = match self {
            Self::Server(c) => c,
            _ => Error::bug_msg("Not server")?,
        };
        let c = ServBehaviour {
            inner: *c,
        };
        Ok(c)
    }
}

pub trait BlockCliBehaviour {
    /// Provide the username to use for authentication. Will only be called once
    /// per session.
    /// If the username needs to change a new connection should be made
    /// – servers often have limits on authentication attempts.
    ///
    fn username(&mut self) -> BhResult<ResponseString>;

    /// Whether to accept a hostkey for the server. The implementation
    /// should compare the key with the key expected for the hostname used.
    fn valid_hostkey(&mut self, key: &PubKey) -> BhResult<bool>;

    /// Get a password to use for authentication returning `Ok(true)`.
    /// Return `Ok(false)` to skip password authentication
    // TODO: having the hostname and username is useful to build a prompt?
    // or we could provide a full prompt as Args
    // TODO: should just return an Option<ResponseString>.
    #[allow(unused)]
    fn auth_password(&mut self, pwbuf: &mut ResponseString) -> BhResult<bool> {
        Ok(false)
    }

    /// Get the next private key to authenticate with. Will not be called
    /// again once returning `HookError::Skip`
    /// The default implementation returns `HookError::Skip`
    fn next_authkey(&mut self) -> BhResult<Option<sign::SignKey>> {
        Ok(None)
    }

    /// Called after authentication has succeeded
    // TODO: perhaps this should be an eventstream not a behaviour?
    fn authenticated(&mut self);

    /// Show a banner sent from a server. Arguments are provided
    /// by the server so could be hazardous, they should be escaped with
    /// [`banner.escape_default()`](core::str::escape_default) or similar.
    /// Language may be empty, is provided by the server.
    #[allow(unused)]
    fn show_banner(&self, banner: &str, language: &str) {
        info!("Got banner:\n{:?}", banner.escape_default());
    }
    // TODO: postauth channel callbacks

    fn open_tcp_forwarded(&self, chan: u32) -> channel::ChanOpened;

    fn open_tcp_direct(&self, chan: u32) -> channel::ChanOpened;
}

pub trait BlockServBehaviour {
    fn hostkeys(&self) -> BhResult<&[&sign::SignKey]>;

    fn have_auth_password(&self, username: &str) -> bool;
    fn have_auth_pubkey(&self, username: &str) -> bool;

    // fn authmethods(&self) -> [AuthMethod];

    fn auth_password(&self, user: &str, password: &str) -> bool;

    /// Returns whether a session can be opened
    fn open_session(&self, chan: u32) -> channel::ChanOpened;

    fn open_tcp_forwarded(&self, chan: u32) -> channel::ChanOpened;

    fn open_tcp_direct(&self, chan: u32) -> channel::ChanOpened;
}
