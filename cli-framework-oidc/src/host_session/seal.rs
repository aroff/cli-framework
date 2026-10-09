//! The sealed payloads of a host session: the session cookie and the
//! short-lived sign-in state cookie.
//!
//! One AES-256-GCM layer, keyed by HKDF-SHA256 from the deployment's session
//! key with a per-purpose `info`, and the purpose as associated data, so a
//! sign-in state can never be opened as a session or the other way round.
//!
//! ```text
//! cookie value = base64url( nonce(12) || ciphertext || tag(16) )
//! ```
//!
//! The session plaintext is a small binary record, not JSON, because the
//! cookie must stay under the browser's 4,096-byte limit with a realm's
//! access and refresh tokens inside. A compact JWS (`h.p.s`) is stored as its
//! three base64url-decoded segments, a quarter smaller, and only when
//! re-encoding gives back the exact same string; any other token is stored as
//! is.

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use zeroize::Zeroizing;

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const SESSION_VERSION: u8 = 1;
const TOKEN_RAW: u8 = 0;
const TOKEN_JWS: u8 = 1;

/// What a sealed value is for. Each has its own key and associated data.
#[derive(Clone, Copy)]
pub(crate) enum Purpose {
    Session,
    SignIn,
}

impl Purpose {
    fn label(self) -> &'static [u8] {
        match self {
            Purpose::Session => b"cli-framework-oidc host session v1",
            Purpose::SignIn => b"cli-framework-oidc host sign-in v1",
        }
    }
}

/// The two AEAD keys derived from one session key.
pub(crate) struct Sealer {
    session: Aes256Gcm,
    sign_in: Aes256Gcm,
}

impl Sealer {
    pub(crate) fn new(session_key: &[u8; 32]) -> Self {
        let derive = |purpose: Purpose| {
            let hkdf = Hkdf::<Sha256>::new(None, session_key);
            let mut key = Zeroizing::new([0u8; 32]);
            hkdf.expand(purpose.label(), key.as_mut())
                .expect("HKDF expand: 32 bytes always fits");
            Aes256Gcm::new_from_slice(key.as_ref()).expect("32-byte key")
        };
        Self {
            session: derive(Purpose::Session),
            sign_in: derive(Purpose::SignIn),
        }
    }

    fn cipher(&self, purpose: Purpose) -> &Aes256Gcm {
        match purpose {
            Purpose::Session => &self.session,
            Purpose::SignIn => &self.sign_in,
        }
    }

    pub(crate) fn seal(&self, purpose: Purpose, plaintext: &[u8]) -> String {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self
            .cipher(purpose)
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: purpose.label(),
                },
            )
            .expect("AES-GCM encryption of an in-memory buffer cannot fail");
        let mut blob = Vec::with_capacity(NONCE_LEN + ct.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        URL_SAFE_NO_PAD.encode(blob)
    }

    /// `None` for anything that isn't a value this key sealed for `purpose`.
    pub(crate) fn open(&self, purpose: Purpose, value: &str) -> Option<Zeroizing<Vec<u8>>> {
        let blob = URL_SAFE_NO_PAD.decode(value).ok()?;
        if blob.len() < NONCE_LEN + TAG_LEN {
            return None;
        }
        let (nonce, ct) = blob.split_at(NONCE_LEN);
        self.cipher(purpose)
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ct,
                    aad: purpose.label(),
                },
            )
            .ok()
            .map(Zeroizing::new)
    }
}

/// The length of the cookie value that sealing `plaintext_len` bytes gives.
pub(crate) fn sealed_len(plaintext_len: usize) -> usize {
    ((NONCE_LEN + plaintext_len + TAG_LEN) * 4).div_ceil(3)
}

/// What a host session cookie holds.
pub(crate) struct SessionRecord {
    /// Random 128-bit session id.
    pub sid: [u8; 16],
    /// Unix seconds of the last request that rewrote the stamp.
    pub last_activity: i64,
    /// Unix seconds when the access token expires.
    pub access_exp: i64,
    /// Unix seconds when the refresh token expires; the session can't outlive it.
    pub refresh_exp: i64,
    /// The caller's binding (for the Apps host: the deployment's data host).
    pub binding: String,
    pub access_token: Zeroizing<String>,
    pub refresh_token: Zeroizing<String>,
}

impl SessionRecord {
    pub(crate) fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::with_capacity(
            64 + self.binding.len() + self.access_token.len() + self.refresh_token.len(),
        ));
        out.push(SESSION_VERSION);
        out.extend_from_slice(&self.sid);
        for n in [self.last_activity, self.access_exp, self.refresh_exp] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        put_bytes(&mut out, self.binding.as_bytes());
        put_token(&mut out, &self.access_token);
        put_token(&mut out, &self.refresh_token);
        out
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader(bytes);
        if r.u8()? != SESSION_VERSION {
            return None;
        }
        let sid: [u8; 16] = r.take(16)?.try_into().ok()?;
        let last_activity = r.i64()?;
        let access_exp = r.i64()?;
        let refresh_exp = r.i64()?;
        let binding = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let access_token = r.token()?;
        let refresh_token = r.token()?;
        if !r.0.is_empty() {
            return None;
        }
        Some(Self {
            sid,
            last_activity,
            access_exp,
            refresh_exp,
            binding,
            access_token,
            refresh_token,
        })
    }
}

/// The plaintext length a session with these parts encodes to, for the size
/// checks at startup and at sign-in.
#[cfg(test)]
pub(crate) fn session_plaintext_len(
    binding: &str,
    access_token: &str,
    refresh_token: &str,
) -> usize {
    SESSION_FIXED_LEN + binding.len() + token_len(access_token) + token_len(refresh_token)
}

/// The record's fixed part: version, sid, three stamps and the binding's length.
pub(crate) const SESSION_FIXED_LEN: usize = 1 + 16 + 24 + 2;

#[cfg(test)]
fn token_len(token: &str) -> usize {
    match jws_segments(token) {
        Some(segs) => 1 + segs.iter().map(|s| 2 + s.len()).sum::<usize>(),
        None => 1 + 2 + token.len(),
    }
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    let len = u16::try_from(b.len()).expect("a cookie field is far below 64 KiB");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(b);
}

fn put_token(out: &mut Vec<u8>, token: &str) {
    match jws_segments(token) {
        Some(segs) => {
            out.push(TOKEN_JWS);
            for s in segs.iter() {
                put_bytes(out, s);
            }
        }
        None => {
            out.push(TOKEN_RAW);
            put_bytes(out, token.as_bytes());
        }
    }
}

/// The three decoded segments of a compact JWS, when they re-encode to exactly
/// `token`.
fn jws_segments(token: &str) -> Option<[Zeroizing<Vec<u8>>; 3]> {
    let mut parts = token.split('.');
    let (h, p, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let dec = |part: &str| -> Option<Zeroizing<Vec<u8>>> {
        let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(part).ok()?);
        (URL_SAFE_NO_PAD.encode(bytes.as_slice()) == part && bytes.len() <= u16::MAX as usize)
            .then_some(bytes)
    };
    Some([dec(h)?, dec(p)?, dec(s)?])
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = u16::from_be_bytes(self.take(2)?.try_into().ok()?) as usize;
        self.take(len)
    }
    fn token(&mut self) -> Option<Zeroizing<String>> {
        match self.u8()? {
            TOKEN_RAW => Some(Zeroizing::new(
                String::from_utf8(self.bytes()?.to_vec()).ok()?,
            )),
            TOKEN_JWS => {
                let h = URL_SAFE_NO_PAD.encode(self.bytes()?);
                let p = URL_SAFE_NO_PAD.encode(self.bytes()?);
                let s = URL_SAFE_NO_PAD.encode(self.bytes()?);
                Some(Zeroizing::new(format!("{h}.{p}.{s}")))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(at: &str, rt: &str) -> SessionRecord {
        SessionRecord {
            sid: [7; 16],
            last_activity: 1_700_000_000,
            access_exp: 1_700_000_300,
            refresh_exp: 1_700_001_800,
            binding: "web-meridis.faseinfra.net".into(),
            access_token: Zeroizing::new(at.into()),
            refresh_token: Zeroizing::new(rt.into()),
        }
    }

    /// A JWS round-trips exactly through its packed form; other tokens are kept raw.
    #[test]
    fn tokens_round_trip_exactly() {
        let jws = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJhIn0.c2lnbmF0dXJl";
        for (at, rt) in [
            (jws, "opaque-refresh-token"),
            ("not.a.jws=", jws),
            ("a.b", "x.y.z.w"),
            // Non-canonical base64url (trailing bits set) must not be packed.
            ("eyJhIjoxfQ.eyJiIjoyfR.AB", "eyJhIjoxfQ.eyJiIjoyfR.AC"),
        ] {
            let r = record(at, rt);
            let bytes = r.encode();
            assert_eq!(bytes.len(), session_plaintext_len(&r.binding, at, rt));
            let back = SessionRecord::decode(&bytes).unwrap();
            assert_eq!(back.access_token.as_str(), at);
            assert_eq!(back.refresh_token.as_str(), rt);
            assert_eq!(back.sid, r.sid);
            assert_eq!(back.binding, r.binding);
            assert_eq!(
                (back.last_activity, back.access_exp, back.refresh_exp),
                (r.last_activity, r.access_exp, r.refresh_exp)
            );
        }
    }

    /// A packed JWS is smaller than the string it came from.
    #[test]
    fn packing_saves_a_quarter() {
        let seg = URL_SAFE_NO_PAD.encode([0x5au8; 900]);
        let jws = format!("{seg}.{seg}.{seg}");
        assert!(token_len(&jws) * 4 < jws.len() * 3 + 40);
    }

    /// Sealed values open only with the same key and purpose; truncation and
    /// bit flips are refused; the size formula matches.
    #[test]
    fn seal_and_open() {
        let a = Sealer::new(&[1; 32]);
        let b = Sealer::new(&[2; 32]);
        for len in [0, 1, 2, 3, 100, 2048] {
            let msg = vec![9u8; len];
            let sealed = a.seal(Purpose::Session, &msg);
            assert_eq!(sealed.len(), sealed_len(len));
            assert_eq!(
                a.open(Purpose::Session, &sealed).unwrap().as_slice(),
                &msg[..]
            );
            assert!(a.open(Purpose::SignIn, &sealed).is_none());
            assert!(b.open(Purpose::Session, &sealed).is_none());
        }
        let sealed = a.seal(Purpose::Session, b"hello");
        let mut raw = URL_SAFE_NO_PAD.decode(&sealed).unwrap();
        raw[NONCE_LEN] ^= 1;
        assert!(a
            .open(Purpose::Session, &URL_SAFE_NO_PAD.encode(&raw))
            .is_none());
        assert!(a.open(Purpose::Session, &sealed[..10]).is_none());
        assert!(a.open(Purpose::Session, "!!!").is_none());
    }

    /// Trailing bytes or an unknown version are refused.
    #[test]
    fn decode_is_strict() {
        let mut bytes = record("a", "b").encode().to_vec();
        bytes.push(0);
        assert!(SessionRecord::decode(&bytes).is_none());
        let mut bytes = record("a", "b").encode().to_vec();
        bytes[0] = 9;
        assert!(SessionRecord::decode(&bytes).is_none());
        assert!(SessionRecord::decode(&[]).is_none());
    }
}
