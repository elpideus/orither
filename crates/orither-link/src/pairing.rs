//! Introducing two devices that have never met.
//!
//! The invite token is everything both sides share, so both derive a throwaway
//! endpoint from it: the host listens as that endpoint and the joiner dials it.
//! Over that channel, each device hands over its permanent identity. Nothing
//! but the token is needed, so pairing works across networks exactly like it
//! does on a LAN, and a short typed code and a QR code are the same flow.
//!
//! The order of saving matters: the host saves before it replies and the joiner
//! saves before it hangs up. Each side starts dialing only after the other has
//! saved, so neither can be refused as a stranger during the handover.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow, bail, ensure};
use argon2::{Algorithm, Argon2, Params, Version};
use iroh::{Endpoint, EndpointId, SecretKey, endpoint::presets};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::oneshot,
    time::{sleep, timeout},
};

use crate::{
    frame,
    store::{Device, PeerId},
};

const ALPN: &[u8] = b"orither/pair/1";
const SALT: &[u8] = b"orither-pairing-v1";
const CODE_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_LEN: usize = 8;
const QR_LEN: usize = 26;
const URI_PREFIX: &str = "orither://pair/";

const INVITE_LIFETIME: Duration = Duration::from_secs(300);
const JOIN_LIFETIME: Duration = Duration::from_secs(45);
const MAX_ATTEMPTS: usize = 5;

/// One pairing invitation, shown as both a short code and a QR code.
#[derive(Clone, Debug, Serialize)]
pub struct Invite {
    /// Short and typeable, for example `K7QM-2XPD`.
    pub code: String,
    /// Longer, unguessable, and encoded in the QR code.
    pub uri: String,
    pub expires_in_secs: u64,
}

impl Invite {
    pub(crate) fn tokens(&self) -> [String; 2] {
        [normalize(&self.code), normalize(&self.uri)]
    }

    pub(crate) fn generate() -> Self {
        let code = random_token(CODE_LEN);
        Self {
            code: format!("{}-{}", &code[..CODE_LEN / 2], &code[CODE_LEN / 2..]),
            uri: format!("{URI_PREFIX}{}", random_token(QR_LEN)),
            expires_in_secs: INVITE_LIFETIME.as_secs(),
        }
    }
}

fn random_token(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len).map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char).collect()
}

/// Reduces what a person typed or a camera read to its canonical form.
fn normalize(input: &str) -> String {
    input
        .trim()
        .trim_start_matches(URI_PREFIX)
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            'U' => 'V',
            other => other,
        })
        .collect()
}

struct Keys {
    endpoint: SecretKey,
    mac: [u8; 32],
}

/// Stretches the token so that a short code resists guessing for its lifetime.
fn derive(token: &str) -> Result<Keys> {
    let params = Params::new(8 * 1024, 3, 1, Some(64)).map_err(|err| anyhow!("{err}"))?;
    let mut output = [0u8; 64];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(token.as_bytes(), SALT, &mut output)
        .map_err(|err| anyhow!("{err}"))?;
    let (endpoint, mac) = output.split_at(32);
    Ok(Keys {
        endpoint: SecretKey::from_bytes(endpoint.try_into()?),
        mac: mac.try_into()?,
    })
}

#[derive(Serialize, Deserialize)]
struct Introduction {
    id: PeerId,
    device: Device,
    proof: String,
}

/// Binding the identity to the token stops anyone who only saw the throwaway
/// endpoint id from introducing themselves.
fn proof(mac: &[u8; 32], id: &PeerId) -> blake3::Hash {
    blake3::keyed_hash(mac, id.as_bytes())
}

fn check(mac: &[u8; 32], intro: &Introduction) -> Result<()> {
    let expected = proof(mac, &intro.id);
    let given = blake3::Hash::from_hex(&intro.proof).context("malformed proof")?;
    ensure!(expected == given, "wrong pairing code");
    Ok(())
}

async fn throwaway_endpoint(secret: SecretKey, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    let id = secret.public();
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .alpns(alpns)
        .bind()
        .await
        .context("binding pairing endpoint")?;
    if let Ok(mdns) = MdnsAddressLookup::builder().build(id) {
        endpoint.address_lookup()?.add(mdns);
    }
    Ok(endpoint)
}

/// Persists a newly introduced device.
pub(crate) type Save = Arc<dyn Fn(PeerId, Device) + Send + Sync>;

/// Waits for a joiner holding either of `tokens`. Only one can win, and dropping
/// the other end of `cancelled` withdraws the invite.
pub(crate) async fn host(
    tokens: &[String; 2],
    me: PeerId,
    device: &Device,
    save: Save,
    cancelled: oneshot::Receiver<()>,
) -> Result<PeerId> {
    let keys = [derive(&tokens[0])?, derive(&tokens[1])?];
    let code = throwaway_endpoint(keys[0].endpoint.clone(), vec![ALPN.to_vec()]).await?;
    let qr = throwaway_endpoint(keys[1].endpoint.clone(), vec![ALPN.to_vec()]).await?;
    let outcome = tokio::select! {
        outcome = serve_invite(&code, &keys[0].mac, me, device, save.clone()) => outcome,
        outcome = serve_invite(&qr, &keys[1].mac, me, device, save) => outcome,
        () = sleep(INVITE_LIFETIME) => Err(anyhow!("invite expired")),
        _ = cancelled => Err(anyhow!("invite withdrawn")),
    };
    tokio::join!(code.close(), qr.close());
    outcome
}

async fn serve_invite(
    endpoint: &Endpoint,
    mac: &[u8; 32],
    me: PeerId,
    device: &Device,
    save: Save,
) -> Result<PeerId> {
    for _ in 0..MAX_ATTEMPTS {
        let incoming = endpoint.accept().await.context("pairing endpoint closed")?;
        let Ok(accepting) = incoming.accept() else { continue };
        let Ok(conn) = accepting.await else { continue };
        let Ok((mut send, mut recv)) = conn.accept_bi().await else { continue };
        let Ok(joiner) = frame::read::<Introduction>(&mut recv).await else { continue };
        if check(mac, &joiner).is_err() {
            conn.close(1u32.into(), b"wrong code");
            continue;
        }
        save(joiner.id, joiner.device);
        let reply = Introduction {
            id: me,
            device: device.clone(),
            proof: proof(mac, &me).to_hex().to_string(),
        };
        frame::write(&mut send, &reply).await?;
        send.finish()?;
        // The joiner hangs up once it has saved us too.
        conn.closed().await;
        return Ok(joiner.id);
    }
    bail!("too many wrong attempts")
}

/// Redeems an invite. Dials repeatedly because a host publishes its address
/// moments after showing the code.
pub(crate) async fn join(token: &str, me: PeerId, device: &Device, save: Save) -> Result<PeerId> {
    let token = normalize(token);
    ensure!(token.len() == CODE_LEN || token.len() == QR_LEN, "that doesn't look like a pairing code");
    let keys = derive(&token)?;
    let host: EndpointId = keys.endpoint.public();
    let endpoint = throwaway_endpoint(SecretKey::generate(), Vec::new()).await?;
    let attempt = timeout(JOIN_LIFETIME, async {
        loop {
            match redeem(&endpoint, host, &keys.mac, me, device, &save).await {
                Ok(peer) => return Ok(peer),
                Err(err) if err.to_string().contains("wrong") => return Err(err),
                Err(err) => {
                    tracing::debug!("pairing attempt failed: {err:#}");
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    })
    .await;
    endpoint.close().await;
    attempt.map_err(|_| anyhow!("couldn't find that device. Check the code and try again"))?
}

async fn redeem(
    endpoint: &Endpoint,
    host: EndpointId,
    mac: &[u8; 32],
    me: PeerId,
    device: &Device,
    save: &Save,
) -> Result<PeerId> {
    let conn = endpoint.connect(host, ALPN).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let intro = Introduction {
        id: me,
        device: device.clone(),
        proof: proof(mac, &me).to_hex().to_string(),
    };
    frame::write(&mut send, &intro).await?;
    let reply: Introduction = frame::read(&mut recv).await.context("wrong code or expired invite")?;
    check(mac, &reply)?;
    save(reply.id, reply.device);
    conn.close(0u32.into(), b"paired");
    Ok(reply.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_mistakes_are_forgiven() {
        assert_eq!(normalize(" k7qm-2xpd "), "K7QM2XPD");
        assert_eq!(normalize("oIlu"), "011V");
        assert_eq!(normalize("orither://pair/ab-cd"), "ABCD");
    }

    #[test]
    fn invites_normalize_to_their_tokens() {
        let invite = Invite::generate();
        let [code, qr] = invite.tokens();
        assert_eq!(code.len(), CODE_LEN);
        assert_eq!(qr.len(), QR_LEN);
    }

    #[test]
    fn proofs_require_the_token() {
        let keys = derive("K7QM2XPD").unwrap();
        let other = derive("K7QM2XPE").unwrap();
        let id = SecretKey::generate().public();
        let intro = Introduction {
            id,
            device: Device { name: "x".into(), kind: crate::DeviceKind::Phone },
            proof: proof(&other.mac, &id).to_hex().to_string(),
        };
        assert!(check(&keys.mac, &intro).is_err());
    }
}
