#[allow(unused_imports)]
use {
    crate::error::{Error, Result, TrapBug},
    log::{debug, error, info, log, trace, warn},
};

use aes::cipher::{BlockSizeUser, KeyIvInit, KeySizeUser, StreamCipher};
use core::num::Wrapping;
use pretty_hex::PrettyHex;
use core::fmt;

use hmac::{Hmac, Mac};
use sha2::Digest as Sha2DigestForTrait;

use crate::*;
use kex::{self, SessId};
use sshnames::*;
use sshwire::hash_mpint;
use ssh_chapoly::SSHChaPoly;

// TODO: check that Ctr32 is sufficient. Should be OK with SSH rekeying.
type Aes256Ctr32BE = ctr::Ctr32BE<aes::Aes256>;
type HmacSha256 = hmac::Hmac<sha2::Sha256>;

// RFC4253 Section 6. Including length u32 length field, excluding MAC
const SSH_MIN_PACKET_SIZE: usize = 16;
const SSH_MIN_PADLEN: usize = 4;
const SSH_MIN_BLOCK: usize = 8;
pub const SSH_LENGTH_SIZE: usize = 4;
pub const SSH_PAYLOAD_START: usize = SSH_LENGTH_SIZE + 1;

// TODO: should calculate/check these somehow
/// Largest is aes256ctr
const MAX_IV_LEN: usize = 32;
/// Largest is chacha. Also applies to MAC keys
const MAX_KEY_LEN: usize = 64;

/// Stateful [`Keys`], stores a sequence number as well, a single instance
/// is kept for the entire session.
pub(crate) struct KeyState {
    keys: Keys,
    // Packet sequence numbers. These must be transferred to subsequent KeyState
    // since they don't reset with rekeying.
    seq_encrypt: Wrapping<u32>,
    seq_decrypt: Wrapping<u32>,
}

impl KeyState {
    /// A brand new `KeyState` with no encryption, zero sequence numbers
    pub fn new_cleartext() -> Self {
        KeyState {
            keys: Keys::new_cleartext(),
            seq_encrypt: Wrapping(0),
            seq_decrypt: Wrapping(0),
        }
    }

    /// Updates with new keys, keeping the same sequence numbers
    pub fn rekey(&mut self, keys: Keys) {
        self.keys = keys
    }

    /// Decrypts the first block in the buffer, returning the length.
    pub fn decrypt_first_block(&mut self, buf: &mut [u8]) -> Result<u32, Error> {
        self.keys.decrypt_first_block(buf, self.seq_decrypt.0)
    }

    /// Decrypt bytes 4 onwards of the buffer and validate AEAD Tag or MAC.
    /// Ensures that the packet meets minimum length.
    pub fn decrypt<'b>(&mut self, buf: &'b mut [u8]) -> Result<usize, Error> {
        let e = self.keys.decrypt(buf, self.seq_decrypt.0);
        self.seq_decrypt += 1;
        e
    }

    /// [`buf`] is the entire output buffer to encrypt in place.
    /// payload_len is the length of the payload portion
    /// This is stateful, updating the sequence number.
    pub fn encrypt<'b>(
        &mut self, payload_len: usize, buf: &'b mut [u8],
    ) -> Result<usize, Error> {
        trace!("encrypt p len {}", payload_len);
        let e = self.keys.encrypt(payload_len, buf, self.seq_encrypt.0);
        self.seq_encrypt += 1;
        e
    }

    pub fn size_block_dec(&self) -> usize {
        self.keys.dec.size_block()
    }
}

pub(crate) struct Keys {
    pub(crate) enc: EncKey,
    pub(crate) dec: DecKey,

    pub(crate) integ_enc: IntegKey,
    pub(crate) integ_dec: IntegKey,
}

impl Keys {
    fn new_cleartext() -> Self {
        Keys {
            enc: EncKey::NoCipher,
            dec: DecKey::NoCipher,
            integ_enc: IntegKey::NoInteg,
            integ_dec: IntegKey::NoInteg,
        }
    }

    pub fn new_from(
        k: &[u8], h: &SessId, sess_id: &SessId, algos: &kex::Algos,
    ) -> Result<Self, Error> {
        let mut key = [0u8; MAX_KEY_LEN];
        let mut iv = [0u8; MAX_IV_LEN];
        let mut hash = algos.kex.hash();

        let (iv_e, iv_d, k_e, k_d, i_e, i_d) = if algos.is_client {
            ('A', 'B', 'C', 'D', 'E', 'F')
        } else {
            ('B', 'A', 'D', 'C', 'F', 'E')
        };

        let enc = {
            let i = Self::compute_key(
                iv_e,
                algos.cipher_enc.iv_len(),
                &mut iv,
                &mut hash,
                k,
                h,
                sess_id,
            )?;
            let k = Self::compute_key(
                k_e,
                algos.cipher_enc.key_len(),
                &mut key,
                &mut hash,
                k,
                h,
                sess_id,
            )?;
            EncKey::from_cipher(&algos.cipher_enc, k, i)?
        };

        let dec = {
            let i = Self::compute_key(
                iv_d,
                algos.cipher_dec.iv_len(),
                &mut iv,
                &mut hash,
                k,
                h,
                sess_id,
            )?;
            let k = Self::compute_key(
                k_d,
                algos.cipher_dec.key_len(),
                &mut key,
                &mut hash,
                k,
                h,
                sess_id,
            )?;
            DecKey::from_cipher(&algos.cipher_dec, k, i)?
        };

        let integ_enc = {
            let k = Self::compute_key(
                i_e,
                algos.integ_enc.key_len(),
                &mut key,
                &mut hash,
                k,
                h,
                sess_id,
            )?;
            IntegKey::from_integ(&algos.integ_enc, k)?
        };

        let integ_dec = {
            let k = Self::compute_key(
                i_d,
                algos.integ_enc.key_len(),
                &mut key,
                &mut hash,
                k,
                h,
                sess_id,
            )?;
            IntegKey::from_integ(&algos.integ_dec, k)?
        };

        let o = Ok(Keys { enc, dec, integ_enc, integ_dec });
        o
    }

    /// RFC4253 7.2. `K1 = HASH(K || H || "A" || session_id)` etc
    fn compute_key<'a>(
        letter: char, len: usize, out: &'a mut [u8],
        hash_ctx: &mut dyn digest::DynDigest, k: &[u8], h: &SessId, sess_id: &SessId,
    ) -> Result<&'a [u8], Error> {
        if len > out.len() {
            return Err(Error::bug());
        }
        let w = &mut [0u8; 32];
        debug_assert!(w.len() >= hash_ctx.output_size());
        // two rounds is sufficient with sha256 and current max key
        debug_assert!(2 * hash_ctx.output_size() >= out.len());

        let l = len.min(hash_ctx.output_size());
        let rest = &mut out[..];
        let (k1, rest) = rest.split_at_mut(l);
        let (k2, _) = rest.split_at_mut(len - l);

        hash_ctx.reset();
        hash_mpint(hash_ctx, k);
        hash_ctx.update(h.as_ref());
        hash_ctx.update(&[letter as u8]);
        hash_ctx.update(sess_id.as_ref());
        hash_ctx.finalize_into_reset(w).trap()?;

        // fill first part
        k1.copy_from_slice(&w[..k1.len()]);

        if k2.len() > 0 {
            // generate next block K2 = HASH(K || H || K1)
            hash_mpint(hash_ctx, k);
            hash_ctx.update(h.as_ref());
            hash_ctx.update(k1);
            hash_ctx.finalize_into_reset(w).trap()?;
            k2.copy_from_slice(&w[..k2.len()]);
        }
        Ok(&out[..len])
    }

    /// Decrypts the first block in the buffer, returning the length of the
    /// total SSH packet (including length+mac) which is calculated
    /// from the decrypted first 4 bytes.
    /// Whether bytes `buf[4..block_size]` are decrypted depends on the cipher, they may be
    /// handled later by [`decrypt`]. Bytes `buf[0..4]` may be left unmodified.
    fn decrypt_first_block(
        &mut self, buf: &mut [u8], seq: u32,
    ) -> Result<u32, Error> {
        if buf.len() < self.dec.size_block() {
            return Err(Error::bug());
        }

        let len = match &mut self.dec {
            DecKey::ChaPoly(k) => k.packet_length(seq, buf).trap()?,
            DecKey::Aes256Ctr(a) => {
                a.apply_keystream(&mut buf[..16]);
                u32::from_be_bytes(buf[..SSH_LENGTH_SIZE].try_into().unwrap())
            }
            DecKey::NoCipher => {
                u32::from_be_bytes(buf[..SSH_LENGTH_SIZE].try_into().unwrap())
            }
        };

        let total_len = len
            .checked_add((SSH_LENGTH_SIZE + self.integ_dec.size_out()) as u32)
            .ok_or(Error::BadDecrypt)?;

        Ok(total_len)
    }

    /// Decrypt the whole packet buffer and validate AEAD Tag or MAC.
    /// Returns the payload length.
    /// Ensures that the packet meets minimum length.
    /// The first block_size bytes may have been already decrypted by
    /// [`decrypt_first_block`] depending on the cipher.
    fn decrypt(&mut self, buf: &mut [u8], seq: u32) -> Result<usize, Error> {
        let size_block = self.dec.size_block();
        let size_integ = self.integ_dec.size_out();

        if buf.len() < size_block + size_integ {
            debug!("Bad packet, {} smaller than block size", buf.len());
            return Err(Error::SSHProtoError);
        }
        if buf.len() < SSH_MIN_PACKET_SIZE + size_integ {
            debug!("Bad packet, {} smaller than min packet size", buf.len());
            return Err(Error::SSHProtoError);
        }
        // "MUST be a multiple of the cipher block size".
        // encrypted length for aead ciphers doesn't include the length prefix.
        let sublength = if self.dec.is_aead() { SSH_LENGTH_SIZE } else { 0 };
        let len = buf.len() - size_integ - sublength;

        if len % size_block != 0 {
            debug!("Bad packet, not multiple of block size");
            return Err(Error::SSHProtoError);
        }

        let (data, mac) = buf.split_at_mut(buf.len() - size_integ);

        // ETM modes would check integrity here.

        match &mut self.dec {
            DecKey::ChaPoly(k) => {
                k.decrypt(seq, data, mac).map_err(|_| {
                    info!("Packet integrity failed");
                    Error::BadDecrypt
                })?;
            }
            DecKey::Aes256Ctr(a) => {
                if data.len() > 16 {
                    a.apply_keystream(&mut data[16..]);
                }
            }
            DecKey::NoCipher => {}
        }

        match self.integ_dec {
            IntegKey::ChaPoly => {}
            IntegKey::NoInteg => {}
            IntegKey::HmacSha256(k) => {
                let mut h = HmacSha256::new_from_slice(&k).trap()?;
                h.update(&seq.to_be_bytes());
                h.update(data);
                h.verify_slice(mac).map_err(|_| {
                    info!("Packet integrity failed");
                    Error::BadDecrypt
                })?;
            }
        }

        let padlen = data[SSH_LENGTH_SIZE] as usize;
        if padlen < SSH_MIN_PADLEN {
            debug!("Packet padding too short");
            return Err(Error::SSHProtoError);
        }

        let payload_len = buf
            .len()
            .checked_sub(SSH_LENGTH_SIZE + 1 + size_integ + padlen)
            .ok_or_else(|| {
                debug!("Bad padding length");
                Error::SSHProtoError
            })?;

        Ok(payload_len)
    }

    /// Padding is required to meet
    /// - minimum packet length
    /// - minimum padding size,
    /// - encrypted length being a multiple of block length
    fn calc_encrypt_pad(&self, payload_len: usize) -> usize {
        let size_block = self.enc.size_block();
        // aead ciphers don't include the initial length field in encrypted blocks
        let len =
            1 + payload_len + if self.enc.is_aead() { 0 } else { SSH_LENGTH_SIZE };

        // round padding length upwards so that len is a multiple of block size
        let mut padlen = size_block - len % size_block;

        // need at least 4 bytes padding
        if padlen < SSH_MIN_PADLEN {
            padlen += size_block
        }

        // The minimum size of a packet is 16 (plus mac)
        // We know we already have at least 8 bytes because of blocksize rounding.
        if SSH_LENGTH_SIZE + 1 + payload_len + padlen < SSH_MIN_PACKET_SIZE {
            padlen += size_block;
        }
        padlen
    }

    /// Encrypt a buffer in-place, adding packet size, padding, MAC etc.
    /// Returns the total length.
    /// Ensures that the packet meets minimum and other length requirements.
    fn encrypt(
        &mut self, payload_len: usize, buf: &mut [u8], seq: u32,
    ) -> Result<usize, Error> {
        let size_block = self.enc.size_block();
        let size_integ = self.integ_enc.size_out();
        let padlen = self.calc_encrypt_pad(payload_len);
        // len is everything except the MAC
        let len = SSH_LENGTH_SIZE + 1 + payload_len + padlen;

        if self.enc.is_aead() {
            debug_assert_eq!((len - SSH_LENGTH_SIZE) % size_block, 0);
        } else {
            debug_assert_eq!(len % size_block, 0);
        };

        if len + size_integ > buf.len() {
            error!("Output buffer {} is too small for packet", buf.len());
            return Err(Error::NoRoom);
        }

        buf[SSH_LENGTH_SIZE] = padlen as u8;
        // write the length
        buf[0..SSH_LENGTH_SIZE]
            .copy_from_slice(&((len - SSH_LENGTH_SIZE) as u32).to_be_bytes());
        // write random padding
        let pad_start = SSH_LENGTH_SIZE + 1 + payload_len;
        debug_assert_eq!(pad_start + padlen, len);
        random::fill_random(&mut buf[pad_start..pad_start + padlen])?;

        let (enc, rest) = buf.split_at_mut(len);
        let (mac, _) = rest.split_at_mut(size_integ);

        match self.integ_enc {
            IntegKey::ChaPoly => {}
            IntegKey::NoInteg => {}
            IntegKey::HmacSha256(k) => {
                let mut h = HmacSha256::new_from_slice(&k).trap()?;
                h.update(&seq.to_be_bytes());
                h.update(enc);
                let result = h.finalize();
                mac.copy_from_slice(&result.into_bytes());
            }
        }

        match &mut self.enc {
            EncKey::ChaPoly(k) => {
                k.encrypt(seq, enc, mac).trap()?
            }
            EncKey::Aes256Ctr(a) => {
                a.apply_keystream(enc);
            }
            EncKey::NoCipher => {}
        }

        // ETM modes would go here.

        Ok(len + size_integ)
    }
}

/// Placeholder for a cipher type prior to creating a a [`EncKey`] or [`DecKey`],
/// for use during key setup in [`kex`]
#[derive(Debug, Clone)]
pub(crate) enum Cipher {
    ChaPoly,
    Aes256Ctr,
    // TODO AesGcm etc
}

impl fmt::Display for Cipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = match self {
            Self::ChaPoly => SSH_NAME_CHAPOLY,
            Self::Aes256Ctr => SSH_NAME_AES256_CTR,
        };
        write!(f, "{n}")
    }
}

impl Cipher {
    /// Creates a cipher key by algorithm name. Must be passed a known name.
    pub fn from_name(name: &'static str) -> Result<Self, Error> {
        match name {
            SSH_NAME_CHAPOLY => Ok(Cipher::ChaPoly),
            SSH_NAME_AES256_CTR => Ok(Cipher::Aes256Ctr),
            _ => Err(Error::bug()),
        }
    }

    /// Length in bytes
    pub fn key_len(&self) -> usize {
        match self {
            Cipher::ChaPoly => SSHChaPoly::key_len(),
            Cipher::Aes256Ctr => aes::Aes256::key_size(),
        }
    }

    /// Length in bytes
    pub fn iv_len(&self) -> usize {
        match self {
            Cipher::ChaPoly => 0,
            Cipher::Aes256Ctr => aes::Aes256::block_size(),
        }
    }

    /// Returns the [`Integ`] for this cipher, or None if not aead
    pub fn integ(&self) -> Option<Integ> {
        match self {
            Cipher::ChaPoly => Some(Integ::ChaPoly),
            Cipher::Aes256Ctr => None,
        }
    }
}

pub(crate) enum EncKey {
    ChaPoly(SSHChaPoly),
    Aes256Ctr(Aes256Ctr32BE),
    // AesGcm(Todo?)
    NoCipher,
}

// TODO: could probably unify EncKey and DecKey as "CipherKey".
// Ring had sealing/opening keys which are separate, but RustCrypto
// uses the same structs in both directions.

impl EncKey {
    /// Construct a key
    pub fn from_cipher<'a>(
        cipher: &Cipher, key: &'a [u8], iv: &'a [u8],
    ) -> Result<Self, Error> {
        match cipher {
            Cipher::ChaPoly => {
                Ok(EncKey::ChaPoly(SSHChaPoly::new(key).trap()?))
            }
            Cipher::Aes256Ctr => Ok(EncKey::Aes256Ctr(
                Aes256Ctr32BE::new_from_slices(key, iv).trap()?,
            )),
        }
    }
    pub fn is_aead(&self) -> bool {
        match self {
            EncKey::ChaPoly(_) => true,
            EncKey::Aes256Ctr(_a) => false,
            EncKey::NoCipher => false,
        }
    }
    pub fn size_block(&self) -> usize {
        match self {
            EncKey::ChaPoly(_) => SSH_MIN_BLOCK,
            EncKey::Aes256Ctr(_) => aes::Aes256::block_size(),
            EncKey::NoCipher => SSH_MIN_BLOCK,
        }
    }
}

pub(crate) enum DecKey {
    ChaPoly(SSHChaPoly),
    Aes256Ctr(Aes256Ctr32BE),
    // AesGcm256
    // AesCtr256
    NoCipher,
}

impl DecKey {
    /// Construct a key
    pub fn from_cipher<'a>(
        cipher: &Cipher, key: &'a [u8], iv: &'a [u8],
    ) -> Result<Self, Error> {
        match cipher {
            Cipher::ChaPoly => {
                Ok(DecKey::ChaPoly(SSHChaPoly::new(key).trap()?))
            }
            Cipher::Aes256Ctr => Ok(DecKey::Aes256Ctr(
                Aes256Ctr32BE::new_from_slices(key, iv).trap()?,
            )),
        }
    }
    pub fn is_aead(&self) -> bool {
        match self {
            DecKey::ChaPoly(_) => true,
            DecKey::Aes256Ctr(_a) => false,
            DecKey::NoCipher => false,
        }
    }
    pub fn size_block(&self) -> usize {
        match self {
            DecKey::ChaPoly(_) => SSH_MIN_BLOCK,
            DecKey::Aes256Ctr(_) => aes::Aes256::block_size(),
            DecKey::NoCipher => SSH_MIN_BLOCK,
        }
    }
}

/// Placeholder for a [`IntegKey`] type prior to keying. For use during key setup in [`kex`]
#[derive(Debug, Clone)]
pub(crate) enum Integ {
    ChaPoly,
    HmacSha256,
    // aesgcm?
}

impl Integ {
    /// Matches a MAC name. Should not be called for AEAD ciphers, instead use [`EncKey::integ`] etc
    pub fn from_name(name: &'static str) -> Result<Self, Error> {
        match name {
            SSH_NAME_HMAC_SHA256 => Ok(Integ::HmacSha256),
            _ => Err(Error::bug()),
        }
    }
    /// length in bytes
    fn key_len(&self) -> usize {
        match self {
            Integ::ChaPoly => 0,
            Integ::HmacSha256 => 32,
        }
    }
}

impl fmt::Display for Integ {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = match self {
            Self::ChaPoly => SSH_NAME_CHAPOLY,
            Self::HmacSha256 => SSH_NAME_HMAC_SHA256,
        };
        write!(f, "{n}")
    }
}

pub(crate) enum IntegKey {
    ChaPoly,
    HmacSha256([u8; 32]),
    // aesgcm?
    // Sha2Hmac ?
    NoInteg,
}

impl IntegKey {
    pub fn from_integ<'a>(integ: &Integ, key: &'a [u8]) -> Result<Self, Error> {
        match integ {
            Integ::ChaPoly => Ok(IntegKey::ChaPoly),
            Integ::HmacSha256 => {
                Ok(IntegKey::HmacSha256(key.try_into().trap()?))
            }
        }
    }
    pub fn size_out(&self) -> usize {
        match self {
            IntegKey::ChaPoly => SSHChaPoly::tag_len(),
            IntegKey::HmacSha256(_) => sha2::Sha256::output_size(),
            IntegKey::NoInteg => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::doorlog::*;
    use crate::encrypt::*;
    use crate::error::Error;
    use crate::sshnames::SSH_NAME_CURVE25519;
    #[allow(unused_imports)]
    use pretty_hex::PrettyHex;

    // setting `corrupt` tests that incorrect mac is detected
    fn do_roundtrips(keys: &mut KeyState, corrupt: bool) {
        // for i in 0usize..40 {
        for i in 0usize..40 {
            let mut v: std::vec::Vec<u8> = (0u8..i as u8 + 60).collect();
            let orig_payload = v[SSH_PAYLOAD_START..SSH_PAYLOAD_START + i].to_vec();

            let written = keys.encrypt(i, v.as_mut_slice()).unwrap();

            v.truncate(written);

            if corrupt {
                // flip a bit of the payload
                v[SSH_PAYLOAD_START] ^= 4;
            }

            let l = keys.decrypt_first_block(v.as_mut_slice()).unwrap() as usize;
            assert_eq!(l, v.len());

            let dec = keys.decrypt(v.as_mut_slice());

            if corrupt {
                assert!(matches!(dec, Err(Error::BadDecrypt)));
                return;
            }
            let payload_len = dec.unwrap();
            assert_eq!(payload_len, i);
            let dec_payload = v[SSH_PAYLOAD_START..SSH_PAYLOAD_START + i].to_vec();
            assert_eq!(orig_payload, dec_payload);
        }
    }

    #[test]
    fn roundtrip_nocipher() {
        // check padding works
        let mut keys = KeyState::new_cleartext();
        do_roundtrips(&mut keys, false);
    }

    #[test]
    #[should_panic]
    fn roundtrip_nocipher_corrupt() {
        // test the test, cleartext has no mac
        let mut keys = KeyState::new_cleartext();
        do_roundtrips(&mut keys, true);
    }

    #[test]
    fn algo_roundtrips() {
        use sha2::Sha256;
        init_test_log();

        let combos = [
            (Cipher::Aes256Ctr, Integ::HmacSha256),
            (Cipher::ChaPoly, Integ::ChaPoly),
        ];

        for (c, i) in combos {
            let mut algos = kex::Algos {
                kex: kex::SharedSecret::from_name(SSH_NAME_CURVE25519).unwrap(),
                hostsig: sign::SigType::Ed25519,
                cipher_enc: c.clone(),
                cipher_dec: c.clone(),
                integ_enc: i.clone(),
                integ_dec: i.clone(),
                discard_next: false,
                is_client: false,
            };

            trace!("Trying cipher {c:?} integ {i:?}");

            // arbitrary keys
            let hash = algos.kex.hash();
            let h = SessId::from_slice(&Sha256::digest("some exchange hash".as_bytes())).unwrap();
            let sess_id = SessId::from_slice(&Sha256::digest("some sessid".as_bytes())).unwrap();
            let sharedkey = "hello".as_bytes();
            let mut newkeys =
                Keys::new_from(sharedkey, &h, &sess_id, &algos).unwrap();

            // client and server enc/dec keys are derived differently, we need them
            // to match for this test
            algos.is_client = !algos.is_client;
            let newkeys_b = Keys::new_from(sharedkey, &h, &sess_id, &algos).unwrap();
            newkeys.dec = newkeys_b.dec;
            newkeys.integ_dec = newkeys_b.integ_dec;

            let mut keys = KeyState::new_cleartext();
            keys.rekey(newkeys);
            do_roundtrips(&mut keys, false);
            do_roundtrips(&mut keys, true);
        }
    }
}
