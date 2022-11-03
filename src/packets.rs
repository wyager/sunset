//! SSH protocol packets.
//!
//! A [`Packet`] can be encoded/decoded to the
//! SSH Binary Packet Protocol using [`sshwire`].

use core::borrow::BorrowMut;
use core::cell::Cell;
use core::fmt;
#[allow(unused_imports)]
use {
    crate::error::{Error, Result, TrapBug},
    log::{debug, error, info, log, trace, warn},
};

use heapless::String;
use pretty_hex::PrettyHex;

use sunset_sshwire_derive::*;

use crate::*;
use namelist::NameList;
use sshnames::*;
use sshwire::{BinString, TextString, Blob};
use sign::{SigType, OwnedSig};
use sshwire::{SSHEncode, SSHDecode, SSHSource, SSHSink, WireResult, WireError};
use sshwire::{SSHEncodeEnum, SSHDecodeEnum};

// Any `enum` needs to have special handling to select a variant when deserializing.
// This is mostly done with `#[sshwire(...)]` attributes.

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct KexInit<'a> {
    pub cookie: [u8; 16],
    pub kex: NameList<'a>,
    /// A list of signature algorithms
    ///
    /// RFC4253 refers to this as the host key algorithms, but actually they
    /// are signature algorithms.
    pub hostsig: NameList<'a>,
    pub cipher_c2s: NameList<'a>,
    pub cipher_s2c: NameList<'a>,
    pub mac_c2s: NameList<'a>,
    pub mac_s2c: NameList<'a>,
    pub comp_c2s: NameList<'a>,
    pub comp_s2c: NameList<'a>,
    pub lang_c2s: NameList<'a>,
    pub lang_s2c: NameList<'a>,
    pub first_follows: bool,
    pub reserved: u32,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct NewKeys {}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct Ignore {}

/// Named to avoid clashing with [`fmt::Debug`]
#[derive(Debug, SSHEncode, SSHDecode)]
pub struct DebugPacket<'a> {
    pub always_display: bool,
    pub message: TextString<'a>,
    pub lang: &'a str,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct Disconnect<'a> {
    pub reason: u32,
    pub desc: TextString<'a>,
    pub lang: TextString<'a>,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct Unimplemented {
    pub seq: u32,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct KexDHInit<'a> {
    pub q_c: BinString<'a>,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct KexDHReply<'a> {
    pub k_s: Blob<PubKey<'a>>,
    pub q_s: BinString<'a>,
    pub sig: Blob<Signature<'a>>,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct ServiceRequest<'a> {
    pub name: &'a str,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct ServiceAccept<'a> {
    pub name: &'a str,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct UserauthRequest<'a> {
    pub username: TextString<'a>,
    pub service: &'a str,
    pub method: AuthMethod<'a>,
}

/// The method-specific part of a [`UserauthRequest`].
#[derive(Debug, SSHEncode, SSHDecode)]
#[sshwire(variant_prefix)]
pub enum AuthMethod<'a> {
    #[sshwire(variant = SSH_AUTHMETHOD_PASSWORD)]
    Password(MethodPassword<'a>),
    #[sshwire(variant = SSH_AUTHMETHOD_PUBLICKEY)]
    PubKey(MethodPubKey<'a>),
    #[sshwire(variant = SSH_NAME_NONE)]
    None,
    #[sshwire(unknown)]
    Unknown(Unknown<'a>),
}

impl<'a> TryFrom<PubKey<'a>> for AuthMethod<'a> {
    type Error = Error;
    fn try_from(pubkey: PubKey<'a>) -> Result<Self> {
        let sig_algo =
            Signature::sig_name_for_pubkey(&pubkey).trap()?;
        Ok(AuthMethod::PubKey(MethodPubKey {
            sig_algo,
            pubkey: Blob(pubkey),
            sig: None,
        }))
    }
}


#[derive(Debug, SSHEncode)]
#[sshwire(no_variant_names)]
pub enum Userauth60<'a> {
    PkOk(UserauthPkOk<'a>),
    PwChangeReq(UserauthPwChangeReq<'a>),
    // TODO keyboard interactive
}

impl<'de: 'a, 'a> SSHDecode<'de> for Userauth60<'a> {
    fn dec<S>(s: &mut S) -> WireResult<Self>
    where S: SSHSource<'de> {
        match s.ctx().cli_auth_type {
            Some(auth::AuthType::Password) => Ok(Self::PwChangeReq(SSHDecode::dec(s)?)),
            Some(auth::AuthType::PubKey) => Ok(Self::PkOk(SSHDecode::dec(s)?)),
            _ => {
                trace!("Wrong packet state for userauth60");
                return Err(WireError::PacketWrong)
            }
        }
    }
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct UserauthPkOk<'a> {
    pub algo: &'a str,
    pub key: Blob<PubKey<'a>>,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct UserauthPwChangeReq<'a> {
    pub prompt: TextString<'a>,
    pub lang: TextString<'a>,
}

#[derive(SSHEncode, SSHDecode)]
pub struct MethodPassword<'a> {
    pub change: bool,
    pub password: TextString<'a>,
}

// Don't print password
impl<'a> fmt::Debug for MethodPassword<'a>{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MethodPassword")
            .field("change", &self.change)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct MethodPubKey<'a> {
    /// A signature algorithm name (not key algorithm name).
    pub sig_algo: &'a str,
    pub pubkey: Blob<PubKey<'a>>,
    pub sig: Option<Blob<Signature<'a>>>,
}

impl SSHEncode for MethodPubKey<'_> {
    fn enc<S>(&self, s: &mut S) -> WireResult<()>
    where S: SSHSink {
        let force_sig_bool = s.ctx().map_or(false, |c| c.method_pubkey_force_sig_bool);
        let sig = self.sig.is_some() || force_sig_bool;
        sig.enc(s)?;
        self.sig_algo.enc(s)?;
        self.pubkey.enc(s)?;
        self.sig.enc(s)?;
        Ok(())
    }
}

impl<'de: 'a, 'a> SSHDecode<'de> for MethodPubKey<'a> {
    fn dec<S>(s: &mut S) -> WireResult<Self>
    where S: sshwire::SSHSource<'de> {
        let sig = bool::dec(s)?;
        let sig_algo = SSHDecode::dec(s)?;
        let pubkey = SSHDecode::dec(s)?;
        let sig = if sig {
            Some(SSHDecode::dec(s)?)
        } else {
            None
        };
        Ok(Self { sig_algo, pubkey, sig })
    }
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct UserauthFailure<'a> {
    pub methods: NameList<'a>,
    pub partial: bool,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct UserauthSuccess {}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct UserauthBanner<'a> {
    pub message: TextString<'a>,
    pub lang: TextString<'a>,
}

#[derive(SSHEncode, SSHDecode, Debug, Clone, PartialEq)]
#[sshwire(variant_prefix)]
pub enum PubKey<'a> {
    #[sshwire(variant = SSH_NAME_ED25519)]
    Ed25519(Ed25519PubKey<'a>),
    #[sshwire(variant = SSH_NAME_RSA)]
    RSA(RSAPubKey<'a>),
    #[sshwire(unknown)]
    Unknown(Unknown<'a>),
}

impl<'a> PubKey<'a> {
    /// The algorithm name presented. May be invalid.
    pub fn algorithm_name(&self) -> Result<&'a str, &Unknown<'a>> {
        match self {
            PubKey::Ed25519(_) => Ok(SSH_NAME_ED25519),
            PubKey::RSA(_) => Ok(SSH_NAME_RSA),
            PubKey::Unknown(u) => Err(u),
        }
    }

    pub fn matches_openssh(&self, k: &str) -> Result<bool> {
        let k = ssh_key::PublicKey::from_openssh(k)
            .map_err(|_| {
                Error::msg("Unsupported OpenSSH key")
            })?;

        let m = match (k.key_data(), self) {
            (ssh_key::public::KeyData::Ed25519(kssh),
                PubKey::Ed25519(kself)) => {
                kssh.0 == kself.key.0
            }
            _ => false,
        };
        Ok(m)
    }
}


#[derive(Debug, Clone, PartialEq, SSHEncode, SSHDecode)]
pub struct Ed25519PubKey<'a> {
    pub key: BinString<'a>,
}

#[derive(Debug, Clone, PartialEq, SSHEncode, SSHDecode)]
pub struct RSAPubKey<'a> {
    pub e: BinString<'a>,
    pub n: BinString<'a>,
}

// #[cfg(feature = "rsa")]
// impl TryFrom<RsaPubKey<'_> for rsa::RsaPublicKey {
//     fn try_from(value: RsaPubKey<'_>) -> Result<Self, Self::Error> {
//         use rsa::BigUint;
//         rsa::RsaPublickey::new(
//             BigUint::from_bytes_be(n.0),
//             BigUint::from_bytes_be(e.0),
//             )
//         .map_err(|e| {
//             debug!("Bad RSA key: {e}");
//             Error::BadKey
//         })
//     }
// }

#[derive(Debug, SSHEncode,  SSHDecode)]
#[sshwire(variant_prefix)]
pub enum Signature<'a> {
    #[sshwire(variant = SSH_NAME_ED25519)]
    Ed25519(Ed25519Sig<'a>),
    #[sshwire(variant = SSH_NAME_RSA_SHA256)]
    RSA256(RSA256Sig<'a>),
    #[sshwire(unknown)]
    Unknown(Unknown<'a>),
}

impl<'a> Signature<'a> {
    /// The algorithm name presented. May be invalid.
    pub fn algorithm_name(&self) -> Result<&'a str, &Unknown<'a>> {
        match self {
            Signature::Ed25519(_) => Ok(SSH_NAME_ED25519),
            Signature::RSA256(_) => Ok(SSH_NAME_RSA_SHA256),
            Signature::Unknown(u) => Err(u),
        }
    }

    /// Returns the signature algorithm name for a public key.
    /// Returns (`Error::UnknownMethod`) if the PubKey is unknown
    /// Currently can return a unique signature name for a public key
    /// since ssh-rsa isn't supported, only rsa-sha2-256 (as an example)
    pub fn sig_name_for_pubkey(pubkey: &PubKey) -> Result<&'static str> {
        match pubkey {
            PubKey::Ed25519(_) => Ok(SSH_NAME_ED25519),
            PubKey::RSA(_) => Ok(SSH_NAME_RSA_SHA256),
            PubKey::Unknown(u) => {
                warn!("Unknown key type \"{}\"", u);
                Err(Error::UnknownMethod {kind: "key"})
            }
        }
    }

    pub fn sig_type(&self) -> Result<SigType> {
        match self {
            Signature::Ed25519(_) => Ok(SigType::Ed25519),
            Signature::RSA256(_) => Ok(SigType::RSA256),
            Signature::Unknown(u) => {
                warn!("Unknown signature type \"{}\"", u);
                Err(Error::UnknownMethod {kind: "signature" })
            }
        }
    }
}

impl <'a> From<&'a OwnedSig> for Signature<'a> {
    fn from(s: &'a OwnedSig) -> Self {
        match s {
            OwnedSig::Ed25519(e) => Signature::Ed25519(Ed25519Sig { sig: BinString(e) }),
            OwnedSig::RSA256 => todo!(),
        }
    }
}


#[derive(Debug, SSHEncode, SSHDecode)]
pub struct Ed25519Sig<'a> {
    pub sig: BinString<'a>,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct RSA256Sig<'a> {
    pub sig: BinString<'a>,
}

// #[derive(Debug)]
// pub struct GlobalRequest<'a> {
//     name: &'a str,
//     want_reply: bool,
//     request: GlobalRequestMethod<'a>,
// }

// enum GlobalRequestMethod<'a> {
//     TcpipForward<'a>,
//     CancelTcpipForward,
// }

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct ChannelOpen<'a> {
    // channel_type is implicit in ty below
    #[sshwire(variant_name = ty)]
    pub num: u32,
    pub initial_window: u32,
    pub max_packet: u32,
    pub ty: ChannelOpenType<'a>,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub enum ChannelOpenType<'a> {
    #[sshwire(variant = "session")]
    Session,
    #[sshwire(variant = "forwarded-tcpip")]
    ForwardedTcpip(ForwardedTcpip<'a>),
    #[sshwire(variant = "direct-tcpip")]
    DirectTcpip(DirectTcpip<'a>),
    // #[sshwire(variant = "x11")]
    // Session(X11<'a>),
    // #[sshwire(variant = "auth-agent@openssh.com")]
    // Session(Agent<'a>),
    #[sshwire(unknown)]
    Unknown(Unknown<'a>),
}

#[derive(Debug,SSHEncode, SSHDecode)]
pub struct ChannelOpenConfirmation {
    pub num: u32,
    pub sender_num: u32,
    pub initial_window: u32,
    pub max_packet: u32,
}

#[derive(Debug,SSHEncode, SSHDecode)]
pub struct ChannelOpenFailure<'a> {
    pub num: u32,
    pub reason: u32,
    pub desc: TextString<'a>,
    pub lang: &'a str,
}

#[derive(Debug,SSHEncode, SSHDecode)]
pub struct ChannelWindowAdjust {
    pub num: u32,
    pub adjust: u32,
}

#[derive(Debug,SSHEncode, SSHDecode)]
pub struct ChannelData<'a> {
    pub num: u32,
    pub data: BinString<'a>,
}

impl ChannelData<'_> {
    // offset into a packet of the raw data
    pub(crate) fn data_offset(&self) -> usize {
        9
    }
}

#[derive(Debug,SSHEncode, SSHDecode)]
pub struct ChannelDataExt<'a> {
    pub num: u32,
    pub code: u32,
    pub data: BinString<'a>,
}

impl ChannelDataExt<'_> {
    // offset into a packet payload
    pub(crate) fn data_offset(&self) -> usize {
    // offset into a packet of the raw data
        13
    }
}

#[derive(Debug,SSHEncode, SSHDecode)]
pub struct ChannelEof {
    pub num: u32,
}

#[derive(Debug,SSHEncode, SSHDecode)]
pub struct ChannelClose {
    pub num: u32,
}

#[derive(Debug,SSHEncode, SSHDecode)]
pub struct ChannelSuccess {
    pub num: u32,
}

#[derive(Debug,SSHEncode, SSHDecode)]
pub struct ChannelFailure {
    pub num: u32,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct ChannelRequest<'a> {
    pub num: u32,

    // channel_type is implicit in req below
    #[sshwire(variant_name = req)]

    pub want_reply: bool,
    pub req: ChannelReqType<'a>,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub enum ChannelReqType<'a> {
    #[sshwire(variant = "shell")]
    Shell,
    #[sshwire(variant = "exec")]
    Exec(Exec<'a>),
    #[sshwire(variant = "pty-req")]
    Pty(Pty<'a>),
    #[sshwire(variant = "subsystem")]
    Subsystem(Subsystem<'a>),
    #[sshwire(variant = "window-change")]
    WinChange(WinChange),
    #[sshwire(variant = "signal")]
    Signal(Signal<'a>),
    #[sshwire(variant = "exit-status")]
    ExitStatus(ExitStatus),
    #[sshwire(variant = "exit-signal")]
    ExitSignal(ExitSignal<'a>),
    #[sshwire(variant = "break")]
    Break(Break),
    // Other requests that aren't implemented at present:
    // auth-agent-req@openssh.com
    // x11-req
    // env
    // xon-xoff
    #[sshwire(unknown)]
    Unknown(Unknown<'a>),
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct Exec<'a> {
    pub command: TextString<'a>,
}

/// The contents of a `"pty-req"` request. Note that most calls use [`channel::Pty`]
/// rather than this struct.
#[derive(Debug, SSHEncode, SSHDecode)]
pub struct Pty<'a> {
    pub term: TextString<'a>,
    pub cols: u32,
    pub rows: u32,
    pub width: u32,
    pub height: u32,
    pub modes: BinString<'a>,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct Subsystem<'a> {
    pub subsystem: &'a str,
}

#[derive(Debug, Clone, SSHEncode, SSHDecode)]
pub struct WinChange {
    pub cols: u32,
    pub rows: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct Signal<'a> {
    pub sig: &'a str,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct ExitStatus {
    pub status: u32,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct ExitSignal<'a> {
    pub signal: &'a str,
    pub core: bool,
    pub error: TextString<'a>,
    pub lang: &'a str,
}

#[derive(Debug, Clone, SSHEncode, SSHDecode)]
pub struct Break {
    pub length: u32,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct ForwardedTcpip<'a> {
    pub address: TextString<'a>,
    pub port: u32,
    pub origin: TextString<'a>,
    pub origin_port: u32,
}

#[derive(Debug, SSHEncode, SSHDecode)]
pub struct DirectTcpip<'a> {
    pub address: TextString<'a>,
    pub port: u32,
    pub origin: TextString<'a>,
    pub origin_port: u32,
}


// Placeholder for unknown method names. These are sometimes non-fatal and
// need to be handled by the relevant code, for example newly invented pubkey types
// This is deliberately not Serializable, we only receive it.
#[derive(Debug, Clone, PartialEq)]
pub struct Unknown<'a>(pub &'a [u8]);

impl core::fmt::Display for Unknown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Ok(s) = sshwire::try_as_ascii_str(self.0) {
            f.write_str(s)
        } else {
            write!(f, "non-ascii {:?}", self.0.hex_dump())
        }
    }
}

/// State to be passed to decoding.
/// Use this so the parser can select the correct enum variant to decode.
#[derive(Default, Clone, Debug)]
pub struct ParseContext {
    pub cli_auth_type: Option<auth::AuthType>,

    // Used by sign_encode()
    pub method_pubkey_force_sig_bool: bool,

    // Set to true if an unknown variant is encountered.
    // Packet length checks should be omitted in that case.
    pub(crate) seen_unknown: bool,
}

impl ParseContext {
    pub fn new() -> Self {
        ParseContext {
            cli_auth_type: None,
            method_pubkey_force_sig_bool: false,
            seen_unknown: false,
        }
    }
}

/// We have repeated `match` statements for the various packet types, use a macro
macro_rules! messagetypes {
    (
        $( ( $message_num:literal,
            $SpecificPacketVariant:ident,
            $SpecificPacketType:ty,
            $SSH_MESSAGE_NAME:ident,
            $category:ident
            ),
             )*
    ) => {


#[derive(Debug)]
#[repr(u8)]
#[allow(non_camel_case_types)]
pub enum MessageNumber {
    // variants are eg
    // SSH_MSG_KEXINIT = 20,
    $(
    $SSH_MESSAGE_NAME = $message_num,
    )*
}

impl TryFrom<u8> for MessageNumber {
    type Error = Error;
    fn try_from(v: u8) -> Result<Self> {
        match v {
            // eg
            // 20 = Ok(MessageNumber::SSH_MSG_KEXINIT)
            $(
            $message_num => Ok(MessageNumber::$SSH_MESSAGE_NAME),
            )*
            _ => {
                Err(Error::UnknownPacket { number: v })
            }
        }
    }
}

impl SSHEncode for Packet<'_> {
    fn enc<S>(&self, s: &mut S) -> WireResult<()>
    where S: SSHSink {
        let t = self.message_num() as u8;
        t.enc(s)?;
        match self {
            // eg
            // Packet::KexInit(p) => {
            // ...
            $(
            Packet::$SpecificPacketVariant(p) => {
                p.enc(s)?
            }
            )*
        };
        Ok(())
    }
}

impl<'de: 'a, 'a> SSHDecode<'de> for Packet<'a> {
    fn dec<S>(s: &mut S) -> WireResult<Self>
    where S: SSHSource<'de> {
        let msg_num = u8::dec(s)?;
        let ty = MessageNumber::try_from(msg_num);
        let ty = match ty {
            Ok(t) => t,
            Err(_) => return Err(WireError::UnknownPacket { number: msg_num })
        };

        // Decode based on the message number
        let p = match ty {
            // eg
            // MessageNumber::SSH_MSG_KEXINIT => Packet::KexInit(
            // ...
            $(
            MessageNumber::$SSH_MESSAGE_NAME => Packet::$SpecificPacketVariant(SSHDecode::dec(s)?),
            )*
        };
        Ok(p)
    }
}

/// Top level SSH packet enum
#[derive(Debug)]
pub enum Packet<'a> {
    // eg KexInit(KexInit<'a>),
    $(
    $SpecificPacketVariant($SpecificPacketType),
    )*
}

impl<'a> Packet<'a> {
    pub fn message_num(&self) -> MessageNumber {
        match self {
            // eg
            // Packet::KexInit() => {
            // ..
            $(
            Packet::$SpecificPacketVariant(_) => {
                MessageNumber::$SSH_MESSAGE_NAME
            }
            )*
        }
    }

    pub fn category(&self) -> Category {
        match self {
            // eg
            // Packet::KexInit() => Category::Kex,
            $(
            Packet::$SpecificPacketVariant(_) => Category::$category,
            )*
        }
    }
}

$(
impl<'a> From<$SpecificPacketType> for Packet<'a> {
    fn from(s: $SpecificPacketType) -> Packet<'a> {
        Packet::$SpecificPacketVariant(s)
    }
}
)*

} } // macro

pub enum Category {
    /// Allowed at any time.
    /// TODO: may need to limit some of these during KEX.
    All,
    /// After kexinit, before newkeys complete (other packets are not allowed during
    /// that time).
    Kex,
    /// Post-kex
    Auth,
    /// Post-auth
    Sess,
}

messagetypes![
(1, Disconnect, Disconnect<'a>, SSH_MSG_DISCONNECT, All),
(2, Ignore, Ignore, SSH_MSG_IGNORE, All),
(3, Unimplemented, Unimplemented, SSH_MSG_UNIMPLEMENTED, All),
(4, DebugPacket, DebugPacket<'a>, SSH_MSG_DEBUG, All),
(5, ServiceRequest, ServiceRequest<'a>, SSH_MSG_SERVICE_REQUEST, Auth),
(6, ServiceAccept, ServiceAccept<'a>, SSH_MSG_SERVICE_ACCEPT, Auth),
(20, KexInit, KexInit<'a>, SSH_MSG_KEXINIT, All),
(21, NewKeys, NewKeys, SSH_MSG_NEWKEYS, Kex),
(30, KexDHInit, KexDHInit<'a>, SSH_MSG_KEXDH_INIT, Kex),
(31, KexDHReply, KexDHReply<'a>, SSH_MSG_KEXDH_REPLY, Kex),

(50, UserauthRequest, UserauthRequest<'a>, SSH_MSG_USERAUTH_REQUEST, Auth),
(51, UserauthFailure, UserauthFailure<'a>, SSH_MSG_USERAUTH_FAILURE, Auth),
(52, UserauthSuccess, UserauthSuccess, SSH_MSG_USERAUTH_SUCCESS, Auth),
(53, UserauthBanner, UserauthBanner<'a>, SSH_MSG_USERAUTH_BANNER, Auth),
// One of
// SSH_MSG_USERAUTH_PASSWD_CHANGEREQ
// SSH_MSG_USERAUTH_PK_OK
// SSH_MSG_USERAUTH_INFO_REQUEST
(60, Userauth60, Userauth60<'a>, SSH_MSG_USERAUTH_60, Auth),
// (61, SSH_MSG_USERAUTH_INFO_RESPONSE),

// (80            SSH_MSG_GLOBAL_REQUEST),
// (81            SSH_MSG_REQUEST_SUCCESS),
// (82            SSH_MSG_REQUEST_FAILURE),
(90, ChannelOpen, ChannelOpen<'a>, SSH_MSG_CHANNEL_OPEN, Sess),
(91, ChannelOpenConfirmation, ChannelOpenConfirmation, SSH_MSG_CHANNEL_OPEN_CONFIRMATION, Sess),
(92, ChannelOpenFailure, ChannelOpenFailure<'a>, SSH_MSG_CHANNEL_OPEN_FAILURE, Sess),
(93, ChannelWindowAdjust, ChannelWindowAdjust, SSH_MSG_CHANNEL_WINDOW_ADJUST, Sess),
(94, ChannelData, ChannelData<'a>, SSH_MSG_CHANNEL_DATA, Sess),
(95, ChannelDataExt, ChannelDataExt<'a>, SSH_MSG_CHANNEL_EXTENDED_DATA, Sess),
(96, ChannelEof, ChannelEof, SSH_MSG_CHANNEL_EOF, Sess),
(97, ChannelClose, ChannelClose, SSH_MSG_CHANNEL_CLOSE, Sess),
(98, ChannelRequest, ChannelRequest<'a>, SSH_MSG_CHANNEL_REQUEST, Sess),
(99, ChannelSuccess, ChannelSuccess, SSH_MSG_CHANNEL_SUCCESS, Sess),
(100, ChannelFailure, ChannelFailure, SSH_MSG_CHANNEL_FAILURE, Sess),
];

#[cfg(test)]
mod tests {
    use crate::sunsetlog::init_test_log;
    use crate::packets::*;
    use crate::sshnames::*;
    use crate::sshwire::tests::{assert_serialize_equal, test_roundtrip};
    use crate::sshwire::{packet_from_bytes, write_ssh};
    use crate::{packets, sshwire};
    use pretty_hex::PrettyHex;

    #[test]
    /// check round trip of packet enums is right
    fn packet_type() {
        for i in 0..=255 {
            let ty = packets::MessageNumber::try_from(i);
            if let Ok(ty) = ty {
                assert_eq!(i, ty as u8);
            }
        }
    }

    #[test]
    /// Tests MethodPubKey custom serde
    fn roundtrip_authpubkey() {
        init_test_log();
        // with None sig
        let s = sign::tests::make_ed25519_signkey();
        let p = UserauthRequest {
            username: "matt".into(),
            service: "conn".into(),
            method: s.pubkey().try_into().unwrap(),
        }.into();
        test_roundtrip(&p);

        // again with a near-genuine sig
        let sig = Signature::Ed25519(Ed25519Sig {
            sig: BinString("something".as_bytes()),
        });
        let sig = Some(Blob(sig));
        let p = UserauthRequest {
            username: "matt".into(),
            service: "conn",
            method: s.pubkey().try_into().unwrap(),
        }.into();
        test_roundtrip(&p);
    }

    #[test]
    fn roundtrip_channel_open() {
        init_test_log();
        let p = Packet::ChannelOpen(ChannelOpen {
            num: 111,
            initial_window: 50000,
            max_packet: 20000,
            ty: ChannelOpenType::DirectTcpip(DirectTcpip {
                address: "localhost".into(),
                port: 4444,
                origin: "somewhere".into(),
                origin_port: 0,
            }),
        });
        test_roundtrip(&p);

        let p = Packet::ChannelOpen(ChannelOpen {
            num: 0,
            initial_window: 899,
            max_packet: 14,
            ty: ChannelOpenType::Session,
        });
        test_roundtrip(&p);
    }

    #[test]
    fn unknown_method() {
        init_test_log();
        let p = Packet::ChannelOpen(ChannelOpen {
            num: 0,
            initial_window: 899,
            max_packet: 14,
            ty: ChannelOpenType::Session,
        });
        let mut buf1 = vec![88; 1000];
        let l = write_ssh(&mut buf1, &p).unwrap();
        buf1.truncate(l);
        // change a byte
        buf1[8] = 'X' as u8;
        trace!("broken: {:?}", buf1.hex_dump());
        let ctx = ParseContext::default();
        let p2 = packet_from_bytes(&buf1, &ctx).unwrap();
        trace!("broken: {p2:#?}");
    }

    #[test]
    #[should_panic]
    fn unknown_method_ser() {
        init_test_log();
        let p = Packet::ChannelOpen(ChannelOpen {
            num: 0,
            initial_window: 200000,
            max_packet: 88200,
            ty: ChannelOpenType::Unknown(Unknown(b"audio-stream"))
        });
        let mut buf1 = vec![88; 1000];
        write_ssh(&mut buf1, &p).unwrap();
    }
}