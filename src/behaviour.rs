#[allow(unused_imports)]
use {
    crate::error::{Error, Result, TrapBug},
    log::{debug, error, info, log, trace, warn},
};

use snafu::prelude::*;

use crate::*;
use packets::{ForwardedTcpip,DirectTcpip};
use channel::ChanOpened;
use sshnames::*;
use sshwire::TextString;

// TODO: "Bh" is an ugly abbreviation. Naming is hard.
// How about SSHApp instead? CliApp, ServApp?

// TODO: probably want a special Result here. They probably all want
// Result, it can return an error or other options like Disconnect?
pub type BhResult<T> = core::result::Result<T, BhError>;

#[derive(Debug,Snafu)]
pub enum BhError {
    Fail,
}

/// A stack-allocated string to store responses for usernames or passwords.
// 100 bytes is an arbitrary size.
// TODO this might get replaced with something better
pub type ResponseString = heapless::String<100>;

// TODO: once async functions in traits work with no_std, some of the trait
// methods could become async. Need to decide whether to be Send or not.
//  Tracking Issue for static async fn in traits
// https://github.com/rust-lang/rust/issues/91611
//  And dyn traits
// https://github.com/rust-lang/rust/issues/107011

// TODO: another interim option would to split the async trait methods
// into a separate trait (which impls the non-async trait)

pub enum Behaviour<'a> {
    Client(&'a mut (dyn CliBehaviour)),
    Server(&'a mut (dyn ServBehaviour)),
}

impl<'a, 'b> From<&'a mut (dyn ServBehaviour + 'b)> for Behaviour<'a> {
    fn from(b: &'a mut (dyn ServBehaviour + 'b)) -> Self {
        Self::Server(b)
    }
}

impl<'a, 'b> From<&'a mut (dyn CliBehaviour + 'b)> for Behaviour<'a> {
    fn from(b: &'a mut (dyn CliBehaviour + 'b)) -> Self {
        Self::Client(b)
    }
}

impl<'a> Behaviour<'a> {
    // TODO: make these From<>
    pub fn new_client(b: &'a mut (dyn CliBehaviour + Send)) -> Self {
        Self::Client(b)
    }

    pub fn new_server(b: &'a mut (dyn ServBehaviour)) -> Self {
        Self::Server(b)
    }

    /// Calls either client or server
    pub(crate) fn open_tcp_forwarded(&mut self, chan: ChanHandle,
        t: &ForwardedTcpip) -> channel::ChanOpened {
        match self {
            Self::Client(b) => b.open_tcp_forwarded(chan, t),
            Self::Server(b) => b.open_tcp_forwarded(chan, t),
        }
    }

    /// Calls either client or server
    pub(crate) fn open_tcp_direct(&mut self, chan: ChanHandle,
        t: &DirectTcpip) -> channel::ChanOpened {
        match self {
            Self::Client(b) => b.open_tcp_direct(chan, t),
            Self::Server(b) => b.open_tcp_direct(chan, t),
        }
    }

    /// Calls either client or server
    pub(crate) fn disconnected(&mut self, desc: TextString) {
        match self {
            Self::Client(b) => b.disconnected(desc),
            Self::Server(b) => b.disconnected(desc),
        }
    }

    pub(crate) fn is_client(&self) -> bool {
        matches!(self, Self::Client(_))
    }

    pub(crate) fn client(&mut self) -> Result<&mut dyn CliBehaviour> {
        match self {
            Self::Client(c) => Ok(*c),
            _ => error::PacketWrong.fail()
        }
    }

    pub(crate) fn server(&mut self) -> Result<&mut dyn ServBehaviour> {
        match self {
            Self::Server(c) => Ok(*c),
            _ => error::PacketWrong.fail(),
        }
    }
}

pub trait CliBehaviour {
    /// Provide the user to use for authentication. Will only be called once
    /// per session.
    ///
    /// If the user needs to change a new connection should be made
    /// – servers often have limits on authentication attempts.
    ///
    fn username(&mut self) -> BhResult<ResponseString>;

    /// Whether to accept a hostkey for the server. The implementation
    /// should compare the key with the key expected for the hostname used.
    fn valid_hostkey(&mut self, key: &PubKey) -> BhResult<bool>;

    /// Get a password to use for authentication returning `Ok(true)`.
    /// Return `Ok(false)` to skip password authentication
    // TODO: having the hostname and user is useful to build a prompt?
    // or we could provide a full prompt as Args
    #[allow(unused)]
    fn auth_password(&mut self, pwbuf: &mut ResponseString) -> BhResult<bool> {
        Ok(false)
    }

    /// Get the next private key used to attempt authentication.
    ///
    /// Will not be called again once returning `None`.
    /// The default implementation returns `None`.
    fn next_authkey(&mut self) -> BhResult<Option<sign::SignKey>> {
        Ok(None)
    }

    /// Sign an authentication request with a key held externally
    /// (most likely in an SSH agent).
    ///
    /// `key` is a key previously returned from `next_authkey()`,
    /// it will be one of the `Agent...` variants.
    #[allow(unused)]
    fn agent_sign(&mut self, key: &sign::SignKey, msg: &AuthSigMsg<'_>) -> BhResult<sign::OwnedSig> {
        Err(BhError::Fail)
    }

    /// Called after authentication has succeeded
    // TODO: perhaps this should be an eventstream not a behaviour?
    fn authenticated(&mut self);

    /// Provides the disconnect message sent by a server
    ///
    /// Note that this may not be called in cases where the SSH TCP connection
    /// is simply closed.
    #[allow(unused)]
    fn disconnected(&mut self, desc: TextString) {
    }

    /// Show a banner sent from a server. Arguments are provided
    /// by the server so could be hazardous, `banner` should be escaped with
    /// [`str::escape_default()`](str::escape_default) or similar.
    /// Language may be empty, is provided by the server.

    // This is a `Behaviour` method rather than an [`Event`] because
    // it must be displayed prior to other authentication
    // functions. `Events` may be handled asynchronously so wouldn't
    // guarantee that.
    #[allow(unused)]
    fn show_banner(&self, banner: TextString, language: TextString) {
    }
    // TODO: postauth channel callbacks

    #[allow(unused)]
    fn open_tcp_forwarded(&mut self, chan: ChanHandle, t: &ForwardedTcpip) -> ChanOpened {
        ChanOpened::Failure((ChanFail::SSH_OPEN_UNKNOWN_CHANNEL_TYPE, chan))
    }

    #[allow(unused)]
    fn open_tcp_direct(&mut self, chan: ChanHandle, t: &DirectTcpip) -> ChanOpened {
        ChanOpened::Failure((ChanFail::SSH_OPEN_UNKNOWN_CHANNEL_TYPE, chan))
    }
}

pub trait ServBehaviour {
    // TODO: load keys on demand?
    // at present `async` isn't very useful here, since it can't load keys
    // on demand. perhaps it should have a callback to get key types,
    // then later request a single key.
    // Also could make it take a closure to call with the key, lets it just
    // be loaded on the stack rather than kept in memory for the whole lifetime.
    fn hostkeys(&mut self) -> BhResult<&[sign::SignKey]>;

    #[allow(unused)]
    // TODO: or return a slice of enums
    /// Implementations should take care to avoid leaking user existence
    /// based on timing.
    fn have_auth_password(&self, username: TextString) -> bool {
        false
    }

    #[allow(unused)]
    /// Implementations should take care to avoid leaking user existence
    /// based on timing.
    fn have_auth_pubkey(&self, username: TextString) -> bool {
        false
    }

    #[allow(unused)]
    /// Return true to allow the user to log in with no authentication
    ///
    /// Implementations may need to take care to avoid leaking user existence
    /// based on timing.
    fn auth_unchallenged(&mut self, username: TextString) -> bool {
        false
    }

    #[allow(unused)]
    // TODO: change password
    /// Implementations should perform password hash comparisons
    /// in constant time, using
    /// [`subtle::ConstantTimeEq`](subtle::ConstantTimeEq) or similar.
    ///
    /// Implementations may need to take care to avoid leaking user existence
    /// based on timing.
    fn auth_password(&mut self, username: TextString, password: TextString) -> bool {
        false
    }

    /// Returns true if the pubkey can be used to log in.
    ///
    /// TODO: allow returning pubkey restriction options
    ///
    /// Implementations may need to take care to avoid leaking user existence
    /// based on timing.
    #[allow(unused)]
    fn auth_pubkey(&mut self, username: TextString, pubkey: &PubKey) -> bool {
        false
    }

    /// Returns whether a session can be opened
    fn open_session(&mut self, chan: ChanHandle) -> channel::ChanOpened;

    #[allow(unused)]
    fn open_tcp_forwarded(&mut self, chan: ChanHandle, t: &ForwardedTcpip) -> ChanOpened {
        ChanOpened::Failure((ChanFail::SSH_OPEN_UNKNOWN_CHANNEL_TYPE, chan))
    }

    #[allow(unused)]
    fn open_tcp_direct(&mut self, chan: ChanHandle, t: &DirectTcpip) -> ChanOpened {
        ChanOpened::Failure((ChanFail::SSH_OPEN_UNKNOWN_CHANNEL_TYPE, chan))
    }

    /// Returns a boolean request success status
    #[allow(unused)]
    fn sess_shell(&mut self, chan: ChanNum) -> bool {
        false
    }

    /// Returns a boolean request success status
    #[allow(unused)]
    fn sess_exec(&mut self, chan: ChanNum, cmd: TextString) -> bool {
        false
    }

    /// Returns a boolean request success status
    #[allow(unused)]
    fn sess_pty(&mut self, chan: ChanNum, pty: &Pty) -> bool {
        false
    }

    /// Provides the disconnect message sent by a client
    ///
    /// Note that this may not be called in cases where the SSH TCP connection
    /// is simply closed.
    #[allow(unused)]
    fn disconnected(&mut self, desc: TextString) {
    }
}
