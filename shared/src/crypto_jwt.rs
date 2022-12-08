use axum::{async_trait, extract::FromRequest, body::{HttpBody}, BoxError, http::StatusCode};
use http::{Request, request::Parts};
use hyper::{header::{self, HeaderName}, Method, Uri, HeaderMap};
use jwt_simple::prelude::{Token, RS256PublicKey, RSAPublicKeyLike, RS256KeyPair, Claims, Duration, RSAKeyPairLike, KeyMetadata, Base64, VerificationOptions};
use openssl::base64;
use serde::{Serialize, de::DeserializeOwned, Deserialize};
use serde_json::Value;
use static_init::dynamic;
use tracing::{debug, error, warn};
use crate::{BeamId, errors::SamplyBeamError, crypto, Msg, MsgSigned, MsgEmpty, MsgId, MsgWithBody, config, beam_id::ProxyId};

const ERR_SIG: (StatusCode, &str) = (StatusCode::UNAUTHORIZED, "Signature could not be verified");
// const ERR_CERT: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "Unable to retrieve matching certificate.");
const ERR_BODY: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "Body is invalid.");
const ERR_FROM: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "\"from\" field in message does not match your certificate.");

#[async_trait]
impl<S: Send + Sync, B: HttpBody + Send + Sync,T> FromRequest<S,B> for MsgSigned<T>
where
    // these trait bounds are copied from `impl FromRequest for axum::Json`
    // T: DeserializeOwned,
    // B: axum::body::HttpBody + Send,
    B::Data: Send,
    B::Error: Into<BoxError>,
    B: HttpBody + 'static,
    T: Serialize + DeserializeOwned + MsgWithBody
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(req: Request<B>, _state: &S) -> Result<Self,Self::Rejection> {
        let (parts, body) = req.into_parts();
        let bytes = hyper::body::to_bytes(body).await.map_err(|_| ERR_BODY)?;
        let token_without_extended_signature = std::str::from_utf8(&bytes).map_err(|e| {
            warn!("Unable to parse token_without_extended_signature as UTF-8: {}", e);
            ERR_SIG
        })?;
        verify_with_extended_header(&parts, Some(token_without_extended_signature)).await
    }
}

#[async_trait]
impl<S,B> FromRequest<S,B> for MsgSigned<MsgEmpty>
where
    // these trait bounds are copied from `impl FromRequest for axum::Json`
    // T: DeserializeOwned,
    // B: axum::body::HttpBody + Send,
    // B::Data: Send,
    // B::Error: Into<BoxError>,
    // T: Serialize + DeserializeOwned + Msg
    S: Send + Sync,
    B: Send + 'static,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(req: Request<B>, _state: &S) -> Result<Self, Self::Rejection> {
        let (parts, _) = req.into_parts();
        verify_with_extended_header(&parts, None).await
    }
}

pub async fn extract_jwt(token: &str) -> Result<(crypto::CryptoPublicPortion, RS256PublicKey, jwt_simple::prelude::JWTClaims<Value>), SamplyBeamError> {
    // TODO: Make static/const
    let options = VerificationOptions {
        accept_future: true,
        ..Default::default()
    };

    let metadata = Token::decode_metadata(token)
        .map_err(|e| SamplyBeamError::RequestValidationFailed(format!("Unable to decode JWT metadata: {}", e)))?;
    let serial = metadata.key_id()
        .ok_or_else(|| SamplyBeamError::RequestValidationFailed(format!("Unable to extract certificate serial from JWT. The offending JWT was: {}", token)))?;
    let public = crypto::get_cert_and_client_by_serial_as_pemstr(serial).await
        .ok_or_else(|| SamplyBeamError::VaultError(format!("Unable to retrieve matching certificate for serial \"{}\"", serial)))?;
    let pubkey = RS256PublicKey::from_pem(&public.pubkey)
        .map_err(|e| {
            SamplyBeamError::SignEncryptError(format!("Unable to initialize public key: {}", e))
        })?;
    let content = pubkey.verify_token::<Value>(token, Some(options))
        .map_err(|e| SamplyBeamError::RequestValidationFailed(format!("Unable to verify token and extract claims from JWT: {}", e)))?;
    Ok((public, pubkey, content))
}

async fn verify_with_extended_header<M: Msg + DeserializeOwned>(req: &Parts, token_without_extended_signature: Option<&str>) -> Result<MsgSigned<M>,(StatusCode, &'static str)> {
    let token_with_extended_signature = std::str::from_utf8(req.headers.get(header::AUTHORIZATION).ok_or_else(|| {
        warn!("Missing Authorization header (in verify_with_extended_header)");
        ERR_SIG
        })?
        .as_bytes())
        .map_err(|e| {
            warn!("Unable to parse existing Authorization header (in verify_with_extended_header): {}", e);
            ERR_SIG
            })?;
    let token_with_extended_signature = token_with_extended_signature.trim_start_matches("SamplyJWT ");
    
    let (proxy_public_info, pubkey, mut content_with) 
        = extract_jwt(token_with_extended_signature).await
        .map_err(|e| {
            warn!("Unable to extract JWT: {}. The full JWT was: {}", e, token_with_extended_signature);
            ERR_SIG
        })?;
    
    // Check extra digest

    let custom = content_with.custom.as_object_mut();
    if custom.is_none() {
        warn!("Received a request with empty JWT custom claims");
        return Err(ERR_SIG);
    }
    let custom = custom.unwrap();
    let digest_claimed = custom.remove("extra_fields_digest");
    if digest_claimed.is_none() {
        warn!("Received a request but had an empty digest_claimed");
        return Err(ERR_SIG);
    }
    let digest_claimed = digest_claimed.unwrap();
    if ! digest_claimed.is_string() {
        warn!("Received a request but digest_claimed was not a valid string");
        return Err(ERR_SIG);
    }
    let digest_claimed = digest_claimed.as_str().unwrap();
    let digest_actual = make_extra_fields_digest(&req.method, &req.uri, &req.headers)
        .map_err(|e| {
            warn!("Got error in make_extra_fields_digest: {}", e);
            ERR_SIG
        })?;
    
    if digest_actual != digest_claimed {
        warn!("Digests did not match: expected {}, received {}", digest_claimed, digest_actual);
        return Err(ERR_SIG);
    }

    // TODO: Make static/const
    let options = VerificationOptions {
        accept_future: true,
        ..Default::default()
    };

    let (custom_without, sig) = if let Some(token_without_extended_signature) = token_without_extended_signature {
        // Check if short token matches the long token
        let content_without = pubkey.verify_token::<Value>(token_without_extended_signature, Some(options))
            .map_err(|e| {
                warn!("Unable to verify short token {}: {}", token_without_extended_signature, e);
                ERR_SIG
            })?;
        let custom_without = content_without.custom.as_object();
        if custom_without.is_none() {
            warn!("Upon message verification, encountered an empty custom_without.");
            return Err(ERR_SIG);
        }
        let custom_without = custom_without.unwrap();
        if custom_without != custom {
            warn!("Long token was verified correctly, but short token's content did not match.");
            return Err(ERR_SIG);
        }
        let custom_without = serde_json::from_value::<M>(Value::Object(custom_without.clone()))
            .map_err(|e| {
                warn!("Unable to unpack custom_without to JSON value, returning ERR_BODY: {}", e);
                ERR_BODY
        })?;
        (custom_without, token_without_extended_signature)
    } else {
        // TODO: We need to fetch the message from the custom_with
        let msg = serde_json::from_value::<M>(content_with.custom)
            .map_err(|e| {
                warn!("Unable to unpack custom_without to JSON value, returning ERR_SIG: {}", e);
                ERR_SIG
            })?;
        let msg_empty = MsgEmpty {
            from: msg.get_from().clone(),
        };
        let serialized = serde_json::to_string(&msg_empty).unwrap(); // known input
        (
             serde_json::from_str::<M>(&serialized).unwrap(), // known input
             ""
        )
    };

    // Check if Messages' "from" attribute can be signed by the proxy
    if ! custom_without.get_from().can_be_signed_by(&proxy_public_info.beam_id) {
        warn!("Received messages' \"from\" attribute which should not have been signed by the proxy.");
        return Err(ERR_FROM);
    }
    // TODO: Check if Date header makes sense (replay attacks)

    let msg_signed = MsgSigned{
        msg: custom_without,
        sig: sig.to_string()
    };

    Ok(msg_signed)
}

pub async fn sign_to_jwt(input: impl Serialize) -> Result<String,SamplyBeamError> {
    let json = serde_json::to_value(input)
        .map_err(|e| SamplyBeamError::SignEncryptError(format!("Serialization failed: {}", e)))?;
    let privkey = &config::CONFIG_SHARED_CRYPTO.get().unwrap().privkey_rs256;
    
    let claims = 
        Claims::with_custom_claims::<Value>(json, Duration::from_hours(1)); // TODO: Make variable

    let token = privkey.sign(claims)
        .map_err(|_| SamplyBeamError::SignEncryptError("Unable to sign JWT".into()))?;
    
    Ok(token)
}

pub fn make_extra_fields_digest(method: &Method, uri: &Uri, headers: &HeaderMap) -> Result<String,SamplyBeamError> {
    const HEADERS_TO_SIGN: [HeaderName; 1] = [
        // header::HOST, // Host header differs from proxy to broker
        header::DATE,
    ];

    let mut buf: Vec<u8> = Vec::new();
    buf.append(&mut method.as_str().as_bytes().to_vec());
    buf.append(&mut uri.to_string().as_bytes().to_vec());
    for header in HEADERS_TO_SIGN {
        if let Some(header) = headers.get(header) {
            let mut bytes = header.as_bytes().to_vec();
            buf.append(&mut bytes);
        } else {
            return Err(SamplyBeamError::SignEncryptError("Required header field not present".into()));
        }
    }

    let digest = crypto::hash(&buf)?;
    let digest = base64::encode_block(&digest);

    Ok(digest)
}
