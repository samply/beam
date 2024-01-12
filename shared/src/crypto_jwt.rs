use std::net::{SocketAddr, IpAddr};

use beam_lib::{AppOrProxyId, ProxyId};
use crate::{
    config,
    config_shared::ConfigCrypto,
    crypto::{self, CryptoPublicPortion},
    errors::{CertificateInvalidReason, SamplyBeamError},
    Msg, MsgEmpty, MsgId, MsgSigned,
};
use axum::{async_trait, body::HttpBody, extract::{FromRequest, ConnectInfo, FromRequestParts}, http::StatusCode, BoxError};
use http::{request::Parts, uri::PathAndQuery, Request};
use hyper::{
    header::{self, HeaderName},
    HeaderMap, Method, Uri,
};
use jwt_simple::{
    claims::JWTClaims,
    prelude::{
        Base64, Base64UrlSafeNoPadding, Claims, Duration, KeyMetadata, RS256KeyPair,
        RS256PublicKey, RSAKeyPairLike, RSAPublicKeyLike, Token, VerificationOptions,
    },
    reexports::ct_codecs::Decoder,
};
use once_cell::unsync::Lazy;
use openssl::base64;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use static_init::dynamic;
use tracing::{debug, error, warn, Span, info_span};

const ERR_SIG: (StatusCode, &str) = (StatusCode::UNAUTHORIZED, "Signature could not be verified");
// const ERR_CERT: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "Unable to retrieve matching certificate.");
const ERR_BODY: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "Body is invalid.");
const ERR_FROM: (StatusCode, &str) = (
    StatusCode::BAD_REQUEST,
    "\"from\" field in message does not match your certificate.",
);

#[async_trait]
impl<S: Send + Sync, B: HttpBody + Send + Sync, T> FromRequest<S, B> for MsgSigned<T>
where
    // these trait bounds are copied from `impl FromRequest for axum::Json`
    // T: DeserializeOwned,
    // B: axum::body::HttpBody + Send,
    B::Data: Send,
    B::Error: Into<BoxError>,
    B: HttpBody + 'static,
    T: Serialize + DeserializeOwned + Msg,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(req: Request<B>, _state: &S) -> Result<Self, Self::Rejection> {
        let (mut parts, body) = req.into_parts();
        let bytes = hyper::body::to_bytes(body).await.map_err(|_| ERR_BODY)?;
        let token_without_extended_signature = std::str::from_utf8(&bytes).map_err(|e| {
            warn!(
                "Unable to parse token_without_extended_signature as UTF-8: {}",
                e
            );
            ERR_SIG
        })?;
        verify_with_extended_header(&mut parts, token_without_extended_signature).await
    }
}

pub type Authorized = MsgSigned<MsgEmpty>;

#[tracing::instrument]
pub async fn extract_jwt<T: DeserializeOwned + Serialize>(
    token: &str,
) -> Result<
    (
        crypto::CryptoPublicPortion,
        RS256PublicKey,
        jwt_simple::prelude::JWTClaims<T>,
    ),
    SamplyBeamError,
> {
    let metadata = Token::decode_metadata(token).map_err(|e| {
        SamplyBeamError::RequestValidationFailed(format!("Unable to decode JWT metadata: {}", e))
    })?;
    let public = if let Some(serial) = metadata.key_id() {
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
        let data = Base64UrlSafeNoPadding::decode_to_vec(data, None).map_err(|e| {
            warn!("Failed to b64decode {data:?}. Err: {e}");
            SamplyBeamError::RequestValidationFailed("Invalid JWT in header".to_string())
        })?;
        let json = serde_json::from_slice::<JWTClaims<HeaderClaim>>(&data).map_err(|e| {
            warn!("Failed to decode {data:?} to JwtClaims<HeaderClaims>. Err: {e}");
            SamplyBeamError::RequestValidationFailed("Invalid JWT body in header".to_string())
        })?;
        let proxy_id: ProxyId = json.custom.from.proxy_id();
        let mut certs = crypto::get_all_certs_and_clients_by_cname_as_pemstr(&proxy_id)
            .await
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        // Get newest Certificate
        crypto::get_newest_cert(&mut certs).ok_or(SamplyBeamError::CertificateError(
            CertificateInvalidReason::NoCommonName,
        ))?
    };
    let pubkey = RS256PublicKey::from_pem(&public.pubkey).map_err(|e| {
        SamplyBeamError::SignEncryptError(format!("Unable to initialize public key: {}", e))
    })?;
    let content = pubkey
        .verify_token::<T>(token, Some(JWT_VERIFICATION_OPTIONS.clone()))
        .map_err(|e| {
            SamplyBeamError::RequestValidationFailed(format!(
                "Unable to verify token and extract claims from JWT: {}",
                e
            ))
        })?;
    Ok((public, pubkey, content))
}

pub const JWT_VERIFICATION_OPTIONS: Lazy<VerificationOptions> = Lazy::new(|| VerificationOptions {
    accept_future: true,
    max_token_length: Some(1024 * 1024 * 100), //100MB
    ..Default::default()
});

/// This verifys a Msg from sent to the Broker
/// The Message is encoded in the JWT Claims of the body which is a JWT.
/// There is never really a [`MsgSigned`] involved in Deserializing the message as the signature is just copied from the body JWT.
/// The token is verified by a key derived from the kid of the JWT in the Header which should also match the kid of the body JWT.
pub async fn verify_with_extended_header<M: Msg + DeserializeOwned>(
    req: &mut Parts,
    token_without_extended_signature: &str,
) -> Result<MsgSigned<M>, (StatusCode, &'static str)> {
    let ip = get_ip(req).await;
    let token_with_extended_signature = req.headers
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

    let (proxy_public_info, pubkey, header_claims) =
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
    let msg = pubkey
        .verify_token::<M>(
            token_without_extended_signature,
            Some(JWT_VERIFICATION_OPTIONS.clone()),
        )
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
    crypto_conf: Option<&ConfigCrypto>,
) -> Result<String, SamplyBeamError> {
    let json = serde_json::to_value(input)
        .map_err(|e| SamplyBeamError::SignEncryptError(format!("Serialization failed: {}", e)))?;
    let privkey = if let Some(ConfigCrypto { privkey_rs256, .. }) = crypto_conf {
        privkey_rs256
    } else {
        &config::CONFIG_SHARED_CRYPTO
            .get()
            .expect("If called by GetCertsFromBroker config needs to be provided by param")
            .privkey_rs256
    };

    let claims = Claims::with_custom_claims::<Value>(json, Duration::from_hours(1)); // TODO: Make variable

    let token = privkey
        .sign(claims)
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
    const HEADERS_TO_SIGN: [HeaderName; 1] = [
        // header::HOST, // Host header differs from proxy to broker
        header::DATE,
    ];

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
    let digest = base64::encode_block(&digest);

    Ok(HeaderClaim {
        sig: digest,
        from: from.to_owned(),
    })
}

async fn get_ip(parts: &mut Parts) -> IpAddr {
    let source_ip = ConnectInfo::<SocketAddr>::from_request_parts(parts, &()).await.expect("The server is configured to keep connect info").0.ip();
    const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
    parts.headers
        .get(X_FORWARDED_FOR)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(source_ip)
}
