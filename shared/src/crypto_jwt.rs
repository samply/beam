use axum::{async_trait, extract::{FromRequest, RequestParts}, body::{HttpBody}, BoxError, http::StatusCode};
use hyper::{header::{self, HeaderName}, Method, Uri, HeaderMap};
use jwt_simple::prelude::{Token, RS256PublicKey, RSAPublicKeyLike, RS256KeyPair, Claims, Duration, RSAKeyPairLike, KeyMetadata, Base64};
use openssl::base64;
use serde::{Serialize, de::DeserializeOwned, Deserialize};
use serde_json::Value;
use tracing::{debug, error, warn};
use crate::{ClientId, errors::SamplyBrokerError, crypto, Msg, MsgSigned, MsgEmpty, MsgId, MsgWithBody};

const ERR_SIG: (StatusCode, &'static str) = (StatusCode::UNAUTHORIZED, "Signature could not be verified");
const ERR_CERT: (StatusCode, &'static str) = (StatusCode::BAD_REQUEST, "Unable to retrieve matching certificate.");
const ERR_BODY: (StatusCode, &'static str) = (StatusCode::BAD_REQUEST, "Body is invalid.");
const ERR_FROM: (StatusCode, &'static str) = (StatusCode::BAD_REQUEST, "\"from\" field in message does not match your certificate.");

#[async_trait]
impl<B: HttpBody + Send + Sync,T> FromRequest<B> for MsgSigned<T>
where
    // these trait bounds are copied from `impl FromRequest for axum::Json`
    // T: DeserializeOwned,
    // B: axum::body::HttpBody + Send,
    B::Data: Send,
    B::Error: Into<BoxError>,
    T: Serialize + DeserializeOwned + MsgWithBody
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self,Self::Rejection> {
        let body = req.take_body().unwrap();
        let bytes = hyper::body::to_bytes(body).await.map_err(|_| ERR_BODY)?;
        let token_without_extended_signature = std::str::from_utf8(&bytes).map_err(|_| ERR_SIG)?;
        second_stage(req, Some(token_without_extended_signature)).await
    }
}

#[async_trait]
impl<B: Send + Sync> FromRequest<B> for MsgSigned<MsgEmpty>
where
    // these trait bounds are copied from `impl FromRequest for axum::Json`
    // T: DeserializeOwned,
    // B: axum::body::HttpBody + Send,
    // B::Data: Send,
    // B::Error: Into<BoxError>,
    // T: Serialize + DeserializeOwned + Msg
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self,Self::Rejection> {
        second_stage(req, None).await
    }
}

async fn second_stage<B,T: Msg + DeserializeOwned>(req: &RequestParts<B>, token_without_extended_signature: Option<&str>) -> Result<MsgSigned<T>,(StatusCode, &'static str)> {
    let token_with_extended_signature = std::str::from_utf8(req.headers().get(header::AUTHORIZATION).ok_or(ERR_SIG)?.as_bytes()).map_err(|_| ERR_SIG)?;
    let token_with_extended_signature = token_with_extended_signature.trim_start_matches("SamplyJWT ");
    
    let metadata = Token::decode_metadata(&token_with_extended_signature)
        .map_err(|_| ERR_SIG)?;
    
    let other_client_id = ClientId::try_from(metadata.key_id().unwrap_or_default())
        .map_err(|_| ERR_SIG)?;
    
    let public = crypto::get_cert_and_client_by_cname_as_pemstr(&other_client_id).await; // an empty key won't get found, resulting in the intended error.
    if public.is_none() {
        return Err(ERR_CERT);
    }
    let public = public.unwrap();

    let pubkey = RS256PublicKey::from_pem(&public.pubkey)
        .map_err(|e| {
            error!("Unable to initialize public key: {}", e);
            ERR_CERT
        })?;

    let mut content_with = pubkey.verify_token::<Value>(&token_with_extended_signature, None)
        .map_err(|e| {
            warn!("Unable to verify long token {}: {}", token_with_extended_signature, e);
            ERR_SIG
        })?;
    
    // Check extra digest

    let custom = content_with.custom.as_object_mut();
    if custom.is_none() {
        return Err(ERR_SIG);
    }
    let custom = custom.unwrap();
    let digest_claimed = custom.remove("extra_fields_digest");
    if digest_claimed.is_none() {
        return Err(ERR_SIG);
    }
    let digest_claimed = digest_claimed.unwrap();
    if ! digest_claimed.is_string() {
        return Err(ERR_SIG);
    }
    let digest_claimed = digest_claimed.as_str().unwrap();
    let digest_actual = make_extra_fields_digest(req.method(), req.uri(), req.headers())
        .map_err(|_| ERR_SIG)?;
    
    if ! (digest_actual == digest_claimed) {
        warn!("Digests did not match: expected {}, received {}", digest_claimed, digest_actual);
        return Err(ERR_SIG);
    }

    let (custom_without, sig) = if let Some(token_without_extended_signature) = token_without_extended_signature {
        // Check if short token matches the long token
        let content_without = pubkey.verify_token::<Value>(&token_without_extended_signature, None)
            .map_err(|e| {
                warn!("Unable to verify short token {}: {}", token_without_extended_signature, e);
                ERR_SIG
            })?;
        let custom_without = content_without.custom.as_object();
        if custom_without.is_none() {
            return Err(ERR_SIG);
        }
        let custom_without = custom_without.unwrap();
        if custom_without != custom {
            warn!("Long token was verified correctly, but short token's content did not match.");
            return Err(ERR_SIG);
        }
        let custom_without = serde_json::from_value::<T>(Value::Object(custom_without.clone()))
            .map_err(|_| ERR_BODY)?;
        (custom_without, token_without_extended_signature)
    } else {
        let msg_empty = MsgEmpty {
            id: MsgId::new(),
            from: public.client.clone(),
        };
        let serialized = serde_json::to_string(&msg_empty).unwrap();
        (
             serde_json::from_str::<T>(&serialized).unwrap(),
             ""
        )
    };

    // Check if Messages' "from" attribute actually matches the public.client
    if public.client != *custom_without.get_from() {
        return Err(ERR_FROM);
    }
    // TODO: Check if Date header makes sense (replay attacks)

    let msg_signed = MsgSigned{
        msg: custom_without,
        sig: sig.to_string()
    };

    Ok(msg_signed)
}

pub async fn sign_to_jwt(json: &Value, privkey_pem: &str, my_client_id: &ClientId) -> Result<String,SamplyBrokerError> {
    let key = RS256KeyPair::from_pem(privkey_pem)
        .map_err(|_| SamplyBrokerError::SignEncryptError("Unable to construct private key".into()))?
        .with_key_id(&my_client_id.to_string()); // TODO: Use cert's serial (not common name) as key id
    
    let claims = 
        Claims::with_custom_claims::<Value>(json.clone(), Duration::from_hours(1));

    let token = key.sign(claims)
        .map_err(|_| SamplyBrokerError::SignEncryptError("Unable to sign JWT".into()))?;
    
    Ok(token)
}

pub fn make_extra_fields_digest(method: &Method, uri: &Uri, headers: &HeaderMap) -> Result<String,SamplyBrokerError> {
    const HEADERS_TO_SIGN: [HeaderName; 1] = [
        // header::HOST, // Host header differs from proxy to central
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
            return Err(SamplyBrokerError::SignEncryptError("Required header field not present".into()));
        }
    }

    let digest = crypto::hash(&buf)?;
    let digest = base64::encode_block(&digest);

    Ok(String::from(digest))
}