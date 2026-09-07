use std::net::{IpAddr, SocketAddr};

use crate::{
    crypto::{self, CryptoPublicPortion},
    errors::{CertificateInvalidReason, SamplyBeamError},
    Msg, MsgEmpty, MsgId, MsgSigned,
};
use axum::{
    body::HttpBody,
    extract::{
        Request, {ConnectInfo, FromRequest, FromRequestParts},
    },
    http::{
        header, request::Parts, uri::PathAndQuery, HeaderMap, HeaderName, Method, StatusCode, Uri,
    },
    BoxError, RequestExt,
};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use beam_lib::{AppOrProxyId, ProxyId};
use jsonwebtoken::{
    decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use once_cell::sync::Lazy;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info_span, warn, Span};

const MAX_TOKEN_LENGTH: usize = 1024 * 1024 * 100;
static JWT_VALIDATION: Lazy<Validation> = Lazy::new(|| Validation::new(Algorithm::RS256));

#[derive(Clone, Debug)]
pub struct JwtSigningKey {
    key: EncodingKey,
    key_id: Option<String>,
}

impl JwtSigningKey {
    pub fn from_pem(pem: &[u8]) -> Result<Self, jsonwebtoken::errors::Error> {
        Ok(Self {
            key: EncodingKey::from_rsa_pem(pem)?,
            key_id: None,
        })
    }

    pub fn with_key_id(mut self, key_id: impl Into<String>) -> Self {
        self.key_id = Some(key_id.into());
        self
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JwtClaims<T> {
    pub exp: u64,
    pub iat: u64,
    #[serde(flatten)]
    pub custom: T,
}

fn verify_token<T: DeserializeOwned>(
    token: &str,
    key: &DecodingKey,
) -> Result<JwtClaims<T>, jsonwebtoken::errors::Error> {
    if token.len() > MAX_TOKEN_LENGTH {
        return Err(jsonwebtoken::errors::Error::from(
            jsonwebtoken::errors::ErrorKind::InvalidToken,
        ));
    }
    decode::<JwtClaims<T>>(token, key, &JWT_VALIDATION)
        .map(|data| data.claims)
}

const ERR_SIG: (StatusCode, &str) = (StatusCode::UNAUTHORIZED, "Signature could not be verified");
// const ERR_CERT: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "Unable to retrieve matching certificate.");
const ERR_FROM: (StatusCode, &str) = (
    StatusCode::BAD_REQUEST,
    "\"from\" field in message does not match your certificate.",
);

impl<S: Send + Sync, T> FromRequest<S> for MsgSigned<T>
where
    // these trait bounds are copied from `impl FromRequest for axum::Json`
    // T: DeserializeOwned,
    // B: axum::body::HttpBody + Send,
    T: Serialize + DeserializeOwned + Msg,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(mut req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let mut parts = req.extract_parts().await.expect("Infallible");
        let token_without_extended_signature: String = req.extract().await.map_err(|e| {
            warn!(
                "Unable to parse token_without_extended_signature as UTF-8: {}",
                e
            );
            ERR_SIG
        })?;
        verify_with_extended_header(&mut parts, &token_without_extended_signature).await
    }
}

pub type Authorized = MsgSigned<MsgEmpty>;

#[tracing::instrument]
pub async fn extract_jwt<T: DeserializeOwned + Serialize>(
    token: &str,
) -> Result<(crypto::CryptoPublicPortion, JwtClaims<T>), SamplyBeamError> {
    let metadata = decode_header(token).map_err(|e| {
        SamplyBeamError::RequestValidationFailed(format!("Unable to decode JWT metadata: {}", e))
    })?;
    let public = if let Some(serial) = metadata.kid.as_deref() {
        crypto::get_cert_and_client_by_serial_as_pemstr(serial)
            .await
            .ok_or_else(|| {
                SamplyBeamError::VaultOtherError(format!(
                    "Unable to retrieve matching certificate for serial \"{}\"",
                    serial
                ))
            })?
            .map_err(|e| SamplyBeamError::CertificateError(e))?
    } else {
        // if it does not have a serial in the metadata try to get it by reading the from field in the body
        // this happens, e.g. during proxy initialization before a certificate (serial) is received
        let data = token
            .splitn(3, ".")
            .nth(1)
            .ok_or(SamplyBeamError::RequestValidationFailed(
                "Invalid JWT in header".to_string(),
            ))?;
        let data = URL_SAFE_NO_PAD.decode(data).map_err(|e| {
            warn!("Failed to b64decode {data:?}. Err: {e}");
            SamplyBeamError::RequestValidationFailed("Invalid JWT in header".to_string())
        })?;
        let json = serde_json::from_slice::<JwtClaims<HeaderClaim>>(&data).map_err(|e| {
            warn!("Failed to decode {data:?} to JwtClaims<HeaderClaims>. Err: {e}");
            SamplyBeamError::RequestValidationFailed("Invalid JWT body in header".to_string())
        })?;
        let proxy_id: ProxyId = json.custom.from.proxy_id();
        let certs = crypto::get_all_certs_and_clients_by_cname_as_pemstr(&proxy_id)
            .await
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        // Get newest Certificate
        crypto::get_newest_cert(certs).ok_or(SamplyBeamError::CertificateError(
            CertificateInvalidReason::NoCommonName,
        ))?
    };
    let content = verify_token::<T>(token, &public.cert.jwt_decoding_key).map_err(|e| {
        SamplyBeamError::RequestValidationFailed(format!(
            "Unable to verify token and extract claims from JWT: {}",
            e
        ))
    })?;
    Ok((public, content))
}

/// This verifys a Msg from sent to the Broker
/// The Message is encoded in the JWT Claims of the body which is a JWT.
/// There is never really a [`MsgSigned`] involved in Deserializing the message as the signature is just copied from the body JWT.
/// The token is verified by a key derived from the kid of the JWT in the Header which should also match the kid of the body JWT.
pub async fn verify_with_extended_header<M: Msg + DeserializeOwned>(
    req: &mut Parts,
    token_without_extended_signature: &str,
) -> Result<MsgSigned<M>, (StatusCode, &'static str)> {
    let ip = get_ip(req).await;
    let token_with_extended_signature = req
        .headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| {
            warn!(%ip, "Missing Authorization header");
            ERR_SIG
        })?
        .to_str()
        .map_err(|e| {
            warn!(%ip, "Unable to parse existing Authorization header: {e}");
            ERR_SIG
        })?;
    let token_with_extended_signature =
        token_with_extended_signature.trim_start_matches("SamplyJWT ");

    let (proxy_public_info, header_claims) =
        extract_jwt::<HeaderClaim>(token_with_extended_signature)
            .await
            .map_err(|e| {
                warn!(%ip, "Unable to extract header JWT: {e}. The full JWT was: {token_with_extended_signature}");
                ERR_SIG
            })?;

    Span::current().record("from", header_claims.custom.from.hide_broker());

    // Check extra digest

    let custom = header_claims.custom;
    let digest_claimed = custom.sig;
    let sender_claimed = custom.from;

    // Check if short token matches the long token
    let msg = verify_token::<M>(token_without_extended_signature, &proxy_public_info.cert.jwt_decoding_key)
        .map_err(|e| {
            warn!(
                "Unable to verify short token {}: {}",
                token_without_extended_signature, e
            );
            ERR_SIG
        })?
        .custom;

    let Some((_, sig)) = token_without_extended_signature.rsplit_once('.') else {
        warn!("Cannot split signature from body token");
        return Err(ERR_SIG);
    };
    let sender_actual = msg.get_from();

    // Check if header claims is matching the body token
    let digest_actual =
        make_extra_fields_digest(&req.method, &req.uri, &req.headers, &sig, &sender_actual)
            .map_err(|e| {
                warn!("Got error in make_extra_fields_digest: {}", e);
                ERR_SIG
            })?
            .sig;

    if digest_actual != digest_claimed {
        warn!(
            "Digests did not match: expected {}, received {}",
            digest_claimed, digest_actual
        );
        return Err(ERR_SIG);
    }

    if sender_actual.to_owned() != sender_claimed {
        warn!(
            "Sender did not match: expected {}, received {}",
            sender_claimed, sender_actual
        );
        return Err(ERR_SIG);
    }

    // Check if Messages' "from" attribute can be signed by the proxy
    if !msg.get_from().can_be_signed_by(&proxy_public_info.beam_id) {
        warn!(
            "Received messages' \"from\" attribute which should not have been signed by the proxy."
        );
        return Err(ERR_FROM);
    }
    // TODO: Check if Date header makes sense (replay attacks)

    let msg_signed = MsgSigned {
        msg,
        jwt: token_without_extended_signature.to_string(),
    };
    Ok(msg_signed)
}

pub async fn sign_to_jwt(
    input: impl Serialize,
    privkey: &JwtSigningKey,
) -> Result<String, SamplyBeamError> {
    let custom = serde_json::to_value(input)
        .map_err(|e| SamplyBeamError::SignEncryptError(format!("Serialization failed: {}", e)))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| {
            SamplyBeamError::SignEncryptError(format!("System clock is before UNIX epoch: {e}"))
        })?
        .as_secs();
    let claims = JwtClaims {
        exp: now + 60 * 60,
        iat: now,
        custom,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = privkey.key_id.clone();
    let token = encode(&header, &claims, &privkey.key)
        .map_err(|e| SamplyBeamError::SignEncryptError(format!("Unable to sign JWT: {}", e)))?;

    Ok(token)
}

#[derive(Serialize, Deserialize)]
pub struct HeaderClaim {
    #[serde(rename = "s")] //safes 2 bytes
    sig: String,
    #[serde(rename = "f")] //safes 3 bytes
    from: AppOrProxyId,
}

pub fn make_extra_fields_digest(
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    sig: &str,
    from: &AppOrProxyId,
) -> Result<HeaderClaim, SamplyBeamError> {
    const HEADERS_TO_SIGN: [HeaderName; 1] = [header::DATE];

    let mut buf: Vec<u8> = Vec::new();
    buf.append(&mut method.as_str().as_bytes().to_vec());
    // Only hashing path and query is sufficient because from will contain the name of the broker that should transmit this task
    let p_and_q = uri
        .path_and_query()
        .map(PathAndQuery::as_str)
        .unwrap_or(uri.path());
    buf.append(&mut p_and_q.as_bytes().to_vec());
    for header in HEADERS_TO_SIGN {
        if let Some(header) = headers.get(header) {
            let mut bytes = header.as_bytes().to_vec();
            buf.append(&mut bytes);
        } else {
            return Err(SamplyBeamError::SignEncryptError(
                "Required header field not present".into(),
            ));
        }
    }
    buf.append(&mut sig.as_bytes().to_vec());
    buf.append(&mut from.to_string().as_bytes().to_vec());

    let digest = crypto::hash(&buf)?;
    let digest = STANDARD.encode(digest);

    Ok(HeaderClaim {
        sig: digest,
        from: from.to_owned(),
    })
}

async fn get_ip(parts: &mut Parts) -> IpAddr {
    let source_ip = ConnectInfo::<SocketAddr>::from_request_parts(parts, &())
        .await
        .expect("The server is configured to keep connect info")
        .0
        .ip();
    const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
    parts
        .headers
        .get(X_FORWARDED_FOR)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(source_ip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::{
        encoding::{AsDer, Pkcs8V1Der},
        rsa::{KeyPair, KeySize},
        signature::KeyPair as _,
    };
    use base64::{engine::general_purpose::STANDARD, Engine};

    #[tokio::test]
    async fn jsonwebtoken_uses_aws_lc_rsa_keys() {
        let key_pair = KeyPair::generate(KeySize::Rsa2048).unwrap();
        let private_der = AsDer::<Pkcs8V1Der>::as_der(&key_pair).unwrap();
        let encoded = STANDARD.encode(private_der.as_ref());
        let body = encoded
            .as_bytes()
            .chunks(64)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let private_pem =
            format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n");
        let signing_key = JwtSigningKey::from_pem(private_pem.as_bytes())
            .unwrap()
            .with_key_id("serial");
        let token = sign_to_jwt(serde_json::json!({ "message": "hello" }), &signing_key)
            .await
            .unwrap();

        assert_eq!(
            decode_header(&token).unwrap().kid.as_deref(),
            Some("serial")
        );
        let public_key = key_pair.public_key().as_ref();
        let claims = verify_token::<Value>(&token, &DecodingKey::from_rsa_der(public_key)).unwrap();
        assert_eq!(claims.custom, serde_json::json!({ "message": "hello" }));
    }
}
