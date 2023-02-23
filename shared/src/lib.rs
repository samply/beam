#![allow(unused_imports)]

use axum::async_trait;
use beam_id::{AppId, AppOrProxyId, BeamId, ProxyId};
use crypto_jwt::extract_jwt;
use errors::SamplyBeamError;
use openssl::base64;
use serde_json::{json, Value};
use sha2::Sha256;
use static_init::dynamic;
use tracing::debug;
//use aes_gcm::{NewAead, aead::Aead, Aes256Gcm};
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use itertools::Itertools;
use rsa::{PaddingScheme, PublicKey, RsaPrivateKey, RsaPublicKey};

use std::{
    fmt::Display,
    ops::Deref,
    time::{Duration, Instant, SystemTime},
};

use rand::Rng;
use serde::{
    de::{DeserializeOwned, Visitor},
    Deserialize, Serialize,
};
use std::{collections::HashMap, str::FromStr};
use uuid::Uuid;

pub type MsgId = MyUuid;
pub type MsgType = String;
pub type TaskResponse = String;

pub mod crypto;
pub mod crypto_jwt;
pub mod errors;
pub mod logger;
mod traits;

pub mod config;
pub mod config_shared;
// #[cfg(feature = "config-for-broker")]
pub mod config_broker;
// #[cfg(feature = "config-for-proxy")]
pub mod config_proxy;

pub mod beam_id;
pub mod http_client;
pub mod middleware;

pub mod examples;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MyUuid(Uuid);
impl MyUuid {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for MyUuid {
    fn default() -> Self {
        Self::new()
    }
}
impl Deref for MyUuid {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl From<Uuid> for MyUuid {
    fn from(uuid: Uuid) -> Self {
        MyUuid(uuid)
    }
}
impl TryFrom<&str> for MyUuid {
    type Error = uuid::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let parsed = Uuid::from_str(value)?;
        Ok(Self(parsed))
    }
}

impl Display for MyUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(PartialEq, Eq, Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase", tag = "status")]
pub enum WorkStatus {
    Claimed,
    TempFailed,
    PermFailed,
    Succeeded,
}

impl Display for WorkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            WorkStatus::Claimed => String::from("Claimed"),
            WorkStatus::TempFailed => String::from("Temporary failure"),
            WorkStatus::PermFailed => String::from("Permanent failure"),
            WorkStatus::Succeeded => String::from("Success"),
        };
        f.write_str(&str)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FailureStrategy {
    Discard,
    Retry {
        backoff_millisecs: usize,
        max_tries: usize,
    }, // backoff for Duration and try max. times
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HowLongToBlock {
    pub wait_time: Option<Duration>,
    pub wait_count: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MsgSigned<M: Msg> {
    pub msg: M,
    pub sig: String,
}

impl<M: Msg> MsgSigned<M> {
    pub async fn verify(&self) -> Result<(), SamplyBeamError> {
        // Signature valid?
        let (proxy_public_info, _, content) = extract_jwt(&self.sig).await?;

        // Message content matches token?
        let val = serde_json::to_value(&self.msg)
            .expect("Internal error: Unable to interpret already parsed message to JSON Value.");
        if content.custom != val {
            return Err(SamplyBeamError::RequestValidationFailed(
                "content.custom did not match parsed message.".to_string(),
            ));
        }

        // From field matches CN in certificate?
        if !self.get_from().can_be_signed_by(&proxy_public_info.beam_id) {
            return Err(SamplyBeamError::RequestValidationFailed(format!(
                "{} is not allowed to sign for {}",
                &proxy_public_info.beam_id,
                self.get_from()
            )));
        }
        debug!("Message has been verified succesfully.");
        Ok(())
    }
}

#[dynamic]
pub static EMPTY_VEC_APPORPROXYID: Vec<AppOrProxyId> = Vec::new();

#[derive(Serialize, Deserialize, Debug)]
pub struct MsgEmpty {
    pub from: AppOrProxyId,
}

impl Msg for MsgEmpty {
    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &EMPTY_VEC_APPORPROXYID
    }

    fn get_metadata(&self) -> &Value {
        &json!(null)
    }
}

pub trait EncMsg<M>: Msg + Serialize
where
    M: Msg + DeserializeOwned,
{
    /// Decrypts an encrypted message. Caution: can panic.
    #[allow(clippy::or_fun_call)]
    fn decrypt(
        &self,
        my_id: &AppOrProxyId,
        my_priv_key: &RsaPrivateKey,
    ) -> Result<M, SamplyBeamError> {

        // JSON parsing
        let binding = serde_json::to_value(self).map_err(|e| {
            SamplyBeamError::SignEncryptError(format!(
                "Decryption error: Cannot deserialize message because {}",
                e
            ))
})?;
        let mut encrypted_json = binding
            .as_object()
            .ok_or(SamplyBeamError::SignEncryptError(
                "Decryption error: Cannot deserialize message".into(),
            ))?
            .to_owned();
        let encrypted_field =
            &mut encrypted_json
                .remove("encrypted")
                .ok_or(SamplyBeamError::SignEncryptError(
                    "Decryption error: No encrypted payload found".into(),
                ))?;
        let encrypted = encrypted_field.as_str()
            .ok_or(SamplyBeamError::JsonParseError("field \"encrypted\" does not contain a valid string"))?;
        let encrypted = base64::decode_block(encrypted)
            .map_err(|_| SamplyBeamError::DecryptError("field \"encrypted\" is not base64 encoded"))?;

        let to_array_index: usize = encrypted_json
            .get("to")
            .ok_or(SamplyBeamError::SignEncryptError(
                "Decryption error: 'to' field not readable".into(),
            ))?
            .as_array()
            .ok_or(SamplyBeamError::SignEncryptError(
                "Decryption error: Cannot get adressee array".into(),
            ))?
            .iter()
            .position(|entry| {
                let entry_str = entry
                    .as_str()
                    .expect("Decryption error: Cannot parse 'to' entries");

                let mut matched = entry_str.ends_with(&my_id.to_string());
                matched &= match entry_str.find(&my_id.to_string()) {
                    Some(0) => true, // Begins with id
                    Some(i) => entry_str.chars().nth(i-1) == Some('.'), // Ends with id, but before is a separator (e.g. appId)
                    None => false
                };
                matched
            }) // TODO remove expect!
            .ok_or(SamplyBeamError::SignEncryptError(
                "Decryption error: This client cannot be found in 'to' list".into(),
            ))?;
        let encrypted_decryption_keys = &mut encrypted_json.remove("encryption_keys").ok_or(
            SamplyBeamError::SignEncryptError(
                "Decryption error: Cannot read 'encryption_keys' field".into(),
            ),
        )?;
        let encrypted_decryption_key: Vec<u8> =
            encrypted_decryption_keys
                .as_array()
                .ok_or(SamplyBeamError::SignEncryptError(
                    "Decryption error: Cannot read 'encrypted_keys' array".into(),
                ))?[to_array_index]
                .as_array()
                .ok_or(SamplyBeamError::SignEncryptError(
                    "Decryption error: Encryption key is not readable".into(),
                ))?
                .iter()
                .map(|v| v.as_u64().unwrap() as u8) //TODO unwrap
                .collect();
        // Cryptographic Operations
        let cipher_engine = XChaCha20Poly1305::new_from_slice(&my_priv_key.decrypt(
            rsa::PaddingScheme::new_oaep::<sha2::Sha256>(),
            &encrypted_decryption_key,
        )?)
        .map_err(|e| {
            SamplyBeamError::SignEncryptError(format!(
                "Decryption error: Cannot initialize stream cipher because {}",
                e
            ))
        })?;
        let nonce: XNonce = XNonce::clone_from_slice(&encrypted[0..24]);
        let ciphertext = &encrypted[24..];
        let plaintext = String::from_utf8(
            cipher_engine
                .decrypt(&nonce, ciphertext.as_ref())
                .map_err(|e| {
                    SamplyBeamError::SignEncryptError(format!(
                        "Decryption error: Cannot decrypt payload because {}",
                        e
                    ))
                })?,
        )
        .map_err(|e| {
            SamplyBeamError::SignEncryptError(format!(
                "Decryption error: Invalid UTF8 text in decrypted ciphertext {}",
                e
            ))
        })?;

        //JSON Reassembling
        let mut decrypted_json = encrypted_json; // The "encrypted" field was removed earlier
        let decrypted_elements: Value = serde_json::from_str(&plaintext).map_err(|e| {
            SamplyBeamError::SignEncryptError(format!(
                "Decryption error: Decrypted plaintext invalid because {}",
                e
            ))
        })?;
        let decrypted_elements =
            decrypted_elements
                .as_object()
                .ok_or(SamplyBeamError::SignEncryptError(
                    "Decryption error: Decrypted plaintext invalid".into(),
                ))?;

        for (key, value) in decrypted_elements.to_owned() {
            let old_val = decrypted_json.insert(key, value);
            if old_val.is_some() {
                return Err(SamplyBeamError::SignEncryptError(
                    "Decryption error: Duplicate field in decrypted message".into(),
                ));
            }
        }
        let result: M = serde_json::from_value(serde_json::Value::from(decrypted_json)).or(Err(
            SamplyBeamError::SignEncryptError(
                "Decryption error: Cannot deserialize message".into(),
            ),
        ))?;
        Ok(result)
    }
}

const FIELDS_TO_ENCRYPT: [&'static str; 1]  = ["body"];

pub trait DecMsg<M>: Msg + Serialize
where
    M: Msg + DeserializeOwned,
{
    #[allow(clippy::or_fun_call)]
    fn encrypt(
        &self,
        receivers_public_keys: &Vec<RsaPublicKey>,
    ) -> Result<M, SamplyBeamError> {
        // Generate Symmetric Key and Nonce
        let mut rng = rand::thread_rng();
        let symmetric_key = XChaCha20Poly1305::generate_key(&mut rng);
        let nonce = XChaCha20Poly1305::generate_nonce(&mut rng);

        // Deserialize Msg
        let binding = serde_json::to_value(self).map_err(|e| {
            SamplyBeamError::SignEncryptError(format!("Cannot deserialize message: {}", e))
        })?;
        let cleartext_json = binding
            .as_object()
            .ok_or(SamplyBeamError::SignEncryptError(
                "Cannot deserialize message".into(),
            ))?
            .to_owned();
        
        // Encrypt symmetric key with receivers' public keys
        let (encrypted_keys, err): (Vec<_>, Vec<_>) = receivers_public_keys
            .iter()
            .map(|key| {
                key.encrypt(
                    &mut rng,
                    PaddingScheme::new_oaep::<Sha256>(),
                    symmetric_key.as_slice(),
                )
            })
            .partition_result();
        if !err.is_empty() {
            return Err(SamplyBeamError::SignEncryptError(
                "Encryption error: Cannot encrypt symmetric key".into(),
            ));
        }

        // Retrieve fields to encrypt and remove from msg
        let mut json_to_encrypt = cleartext_json.clone();
        json_to_encrypt.retain(|k, _| FIELDS_TO_ENCRYPT.contains(&k.as_str()));
        let mut encrypted_json = cleartext_json;
        for f in FIELDS_TO_ENCRYPT {
            _ = encrypted_json.remove(f);
        }

        // Add encrypted key to msg
        encrypted_json.insert(
            String::from("encryption_keys"),
            serde_json::Value::from(encrypted_keys),
        );

        // Encrypt fields content
        let cipher = XChaCha20Poly1305::new(&symmetric_key);
        let plain_value = serde_json::Value::from(json_to_encrypt);
        let plaintext = plain_value.to_string();
        let mut ciphertext = cipher.encrypt(&nonce, plaintext.as_ref()).or(Err(
            SamplyBeamError::SignEncryptError("Encryption error: Can not encrypt data.".into()),
        ))?;

        // Prepend Nonce to ciphertext
        let mut nonce_and_ciphertext = nonce.to_vec();
        nonce_and_ciphertext.append(&mut ciphertext);

        let nonce_and_ciphertext = base64::encode_block(&nonce_and_ciphertext);

        // Add Encrypted fields to msg
        encrypted_json.insert(
            String::from("encrypted"),
            serde_json::Value::from(nonce_and_ciphertext),
        );

        let serialized_string = serde_json::Value::from(encrypted_json.clone()).to_string();
        let result: M =
            serde_json::from_str(&serialized_string).or(Err(SamplyBeamError::SignEncryptError(
                "Encryption error: Cannot deserialize message".into(),
            )))?;


        Ok(result)
    }
}

pub trait Msg: Serialize {
    fn get_from(&self) -> &AppOrProxyId;
    fn get_to(&self) -> &Vec<AppOrProxyId>;
    fn get_metadata(&self) -> &Value;
}

pub trait MsgWithBody: Msg {
    // fn get_body(&self) -> &str;
}
impl MsgWithBody for MsgTaskRequest {
    // fn get_body(&self) -> &str {
    //     &self.body
    // }
}
impl MsgWithBody for MsgTaskResult {
    // fn get_body(&self) -> &str {
    //     self.get_body()
    // }
}

impl<M: Msg> Msg for MsgSigned<M> {
    fn get_from(&self) -> &AppOrProxyId {
        self.msg.get_from()
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        self.msg.get_to()
    }

    fn get_metadata(&self) -> &Value {
        self.msg.get_metadata()
    }
}

impl Msg for MsgTaskRequest {
    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &self.to
    }

    fn get_metadata(&self) -> &Value {
        &self.metadata
    }
}

impl Msg for MsgTaskResult {
    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &self.to
    }

    fn get_metadata(&self) -> &Value {
        &self.metadata
    }
}

impl Msg for EncryptedMsgTaskRequest {
    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &self.to
    }

    fn get_metadata(&self) -> &Value {
        &self.metadata
    }
}

impl Msg for EncryptedMsgTaskResult {
    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &self.to
    }

    fn get_metadata(&self) -> &Value {
        &self.metadata
    }
}

// impl From<MsgSigned<MsgTaskRequest>> for MsgTaskRequest {
//     fn from(x: MsgSigned<MsgTaskRequest>) -> Self {
//         x.msg
//     }
// }

// impl From<MsgSigned<MsgTaskResult>> for MsgTaskResult {
//     fn from(x: MsgSigned<MsgTaskResult>) -> Self {
//         x.msg
//     }
// }

mod serialize_time {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde::{self, Deserialize, Deserializer, Serializer};
    use tracing::{debug, error, warn};

    pub fn serialize<S>(time: &SystemTime, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let ttl = match time.duration_since(SystemTime::now()) {
            Ok(v) => v,
            Err(e) => {
                error!("Internal Error: Tried to serialize a task which should have expired and expunged from memory {} seconds ago. Will return TTL=0. Cause: {}", e.duration().as_secs(), e);
                Duration::ZERO
            }
        };
        s.serialize_u64(ttl.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ttl: u64 = u64::deserialize(deserializer)?;
        let expire = SystemTime::now() + Duration::from_secs(ttl);
        debug!("Deserialized u64 {} to time {:?}", ttl, expire);
        Ok(expire)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MsgTaskRequest {
    pub id: MsgId,
    pub from: AppOrProxyId,
    pub to: Vec<AppOrProxyId>,
    pub body: String,
    #[serde(with = "serialize_time", rename = "ttl")]
    pub expire: SystemTime,
    pub failure_strategy: FailureStrategy,
    #[serde(skip)]
    pub results: HashMap<AppOrProxyId, MsgSigned<MsgTaskResult>>,
    pub metadata: Value,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct EncryptedMsgTaskRequest {
    pub id: MsgId,
    pub from: AppOrProxyId,
    pub to: Vec<AppOrProxyId>,
    pub encrypted: Vec<u8>,
    pub encryption_keys: Vec<Vec<u8>>,
    #[serde(with = "serialize_time", rename = "ttl")]
    pub expire: SystemTime,
    pub failure_strategy: FailureStrategy,
    pub metadata: Value,
    #[serde(skip)]
    pub results: HashMap<AppOrProxyId, MsgSigned<EncryptedMsgTaskResult>>,
}

//TODO: Implement EncMsg and DecMsg for all message types
impl EncMsg<MsgTaskRequest> for EncryptedMsgTaskRequest {}
impl DecMsg<EncryptedMsgTaskRequest> for MsgTaskRequest {}

impl EncryptedMsgTaskRequest {
    pub fn id(&self) -> &MsgId {
        &self.id
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MsgTaskResult {
    pub from: AppOrProxyId,
    pub to: Vec<AppOrProxyId>,
    pub task: MsgId,
    #[serde(flatten)]
    pub status: WorkStatus,
    pub body: Option<String>,
    pub metadata: Value,
}
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct EncryptedMsgTaskResult {
    pub from: AppOrProxyId,
    pub to: Vec<AppOrProxyId>,
    pub task: MsgId,
    #[serde(flatten)]
    pub status: WorkStatus,
    pub encrypted: Vec<u8>,
    pub encryption_keys: Vec<Vec<u8>>,
    pub metadata: Value,
}

impl EncMsg<MsgTaskResult> for EncryptedMsgTaskResult {}
impl DecMsg<EncryptedMsgTaskResult> for MsgTaskResult {}

impl MsgWithBody for EncryptedMsgTaskRequest {}
impl MsgWithBody for EncryptedMsgTaskResult {}

pub trait HasWaitId<I: PartialEq> {
    fn wait_id(&self) -> I;
}

impl HasWaitId<MsgId> for MsgTaskRequest {
    fn wait_id(&self) -> MsgId {
        self.id
    }
}

impl HasWaitId<String> for MsgTaskResult {
    fn wait_id(&self) -> String {
        format!("{},{}", self.task, self.from)
    }
}

impl HasWaitId<MsgId> for EncryptedMsgTaskRequest {
    fn wait_id(&self) -> MsgId {
        self.id
    }
}

impl HasWaitId<String> for EncryptedMsgTaskResult {
    fn wait_id(&self) -> String {
        format!("{},{}", self.task, self.from)
    }
}

impl<M> HasWaitId<MsgId> for MsgSigned<M>
where
    M: HasWaitId<MsgId> + Msg,
{
    fn wait_id(&self) -> MsgId {
        self.msg.wait_id()
    }
}

impl<M> HasWaitId<String> for MsgSigned<M>
where
    M: HasWaitId<String> + Msg,
{
    fn wait_id(&self) -> String {
        self.msg.wait_id()
    }
}

impl MsgTaskRequest {
    pub fn id(&self) -> &MsgId {
        &self.id
    }

    pub fn new(
        from: AppOrProxyId,
        to: Vec<AppOrProxyId>,
        body: String,
        failure_strategy: FailureStrategy,
        metadata: serde_json::Value,
    ) -> Self {
        MsgTaskRequest {
            id: MsgId::new(),
            from,
            to,
            body,
            failure_strategy,
            results: HashMap::new(),
            metadata,
            expire: SystemTime::now() + Duration::from_secs(3600),
        }
    }
}

// Don't compare expire, as it is constantly changing.
// Todo Is the comparison of Results nessecary
impl PartialEq for MsgTaskRequest {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.from == other.from
            && self.to == other.to
            && self.body == other.body
            && self.failure_strategy == other.failure_strategy
            && self.results == other.results
            && self.metadata == other.metadata
    }
}
impl Eq for MsgTaskRequest {}

impl PartialEq for EncryptedMsgTaskRequest {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.from == other.from
            && self.to == other.to
            && self.encrypted == other.encrypted
            && self.encryption_keys == other.encryption_keys
            && self.failure_strategy == other.failure_strategy
            && self.metadata == other.metadata
            && self.results == other.results
    }
}
impl Eq for EncryptedMsgTaskRequest {}

#[derive(Debug, Serialize, Deserialize)]
pub struct MsgPing {
    id: MsgId,
    from: AppOrProxyId,
    to: Vec<AppOrProxyId>,
    nonce: [u8; 16],
    metadata: Value,
}

impl MsgPing {
    pub fn new(from: AppOrProxyId, to: AppOrProxyId) -> Self {
        let mut nonce = [0; 16];
        openssl::rand::rand_bytes(&mut nonce)
            .expect("Critical Error: Failed to generate random byte array.");
        MsgPing {
            id: MsgId::new(),
            from,
            to: vec![to],
            nonce,
            metadata: json!(null),
        }
    }
}

impl Msg for MsgPing {
    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &self.to
    }

    fn get_metadata(&self) -> &Value {
        &self.metadata
    }
}

pub fn try_read<T>(map: &HashMap<String, String>, key: &str) -> Option<T>
where
    T: FromStr,
{
    map.get(key).and_then(|value| match value.parse() {
        Ok(v) => Some(v),
        Err(_) => None,
    })
}

#[cfg(test)]
mod tests {
    use crate::beam_id::BrokerId;

    use super::*;

    #[test]
    fn encrypt_decrypt_task() {
        //Create Task
        AppId::set_broker_id("broker.samply.de".to_string());
        let p1_id = AppOrProxyId::AppId(AppId::new("app.proxy1.broker.samply.de").unwrap());
        let p2_id = AppOrProxyId::AppId(AppId::new("app.proxy2.broker.samply.de").unwrap());
        let from = p1_id.clone();
        let to = vec![p1_id.clone(), p2_id.clone()];
        let expiry = SystemTime::now() + Duration::from_secs(60);
        let failure = FailureStrategy::Discard;
        let msg = MsgTaskRequest {
            id: MsgId::new(),
            from,
            to,
            body: "Testbody".to_string(),
            expire: expiry,
            failure_strategy: failure,
            results: HashMap::new(),
            metadata: "".into(),
        };

        //Setup Keypairs
        let mut rng = rand::thread_rng();
        let rsa_length: usize = 2048;
        let p1_private = RsaPrivateKey::new(&mut rng, rsa_length)
            .expect("Failed to generate private key for proxy 1");
        let p2_private = RsaPrivateKey::new(&mut rng, rsa_length)
            .expect("Failed to generate private key for proxy 2");
        let p1_public = RsaPublicKey::from(&p1_private);
        let p2_public = RsaPublicKey::from(&p2_private);

        // Encrypt Message
        let receivers_public_keys = vec![p1_public, p2_public];
        let msg_encr = msg
            .encrypt(&receivers_public_keys)
            .expect("Could not encrypt message");
        // Decrypt for both proxies
        let msg_p1_decr = msg_encr
            .decrypt(&p1_id, &p1_private)
            .expect("Cannot decrypt message");
        let msg_p2_decr = msg_encr
            .decrypt(&p2_id, &p2_private)
            .expect("Cannot decrypt message");

        assert_eq!(msg_p1_decr, msg_p2_decr);
        assert_eq!(msg, msg_p1_decr);
    }

    #[test]
    fn encrypt_decrypt_result() {
        AppId::set_broker_id("broker.samply.de".to_string());
        let p1_id = AppOrProxyId::AppId(AppId::new("app.proxy1.broker.samply.de").unwrap());
        let p2_id = AppOrProxyId::AppId(AppId::new("app.proxy2.broker.samply.de").unwrap());
        let from = p1_id.clone();
        let to = vec![p1_id.clone(), p2_id.clone()];
        let status = WorkStatus::Succeeded;
        let msg = MsgTaskResult {
            from,
            to,
            task: MsgId::new(),
            status,
            body: Some("The result is 55!".to_string()),
            metadata: "".into(),
        };

        //Setup Keypairs
        let mut rng = rand::thread_rng();
        let rsa_length: usize = 2048;
        let p1_private = RsaPrivateKey::new(&mut rng, rsa_length)
            .expect("Failed to generate private key for proxy 1");
        let p2_private = RsaPrivateKey::new(&mut rng, rsa_length)
            .expect("Failed to generate private key for proxy 2");
        let p1_public = RsaPublicKey::from(&p1_private);
        let p2_public = RsaPublicKey::from(&p2_private);

        // Encrypt Message
        let receivers_public_keys = vec![p1_public, p2_public];
        let msg_encr = msg
            .encrypt(&receivers_public_keys)
            .expect("Could not encrypt message");
        // Decrypt for both proxies
        let msg_p1_decr = msg_encr
            .decrypt(&p1_id, &p1_private)
            .expect("Cannot decrypt message");
        let msg_p2_decr = msg_encr
            .decrypt(&p2_id, &p2_private)
            .expect("Cannot decrypt message");

        assert_eq!(msg_p1_decr, msg_p2_decr);
        assert_eq!(msg, msg_p1_decr);
    }
}

impl std::fmt::Debug for EncryptedMsgTaskRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedMsgTaskRequest")
            .field("id", &self.id)
            .field("from", &self.from)
            .field("to", &self.to)
            .field("encrypted (#bytes)", &self.encrypted.len())
            .field("encryption_keys (#)", &self.encryption_keys.len())
            .field("expire", &self.expire)
            .field("failure_strategy", &self.failure_strategy)
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl std::fmt::Debug for EncryptedMsgTaskResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedMsgTaskResult")
        .field("from", &self.from)
        .field("to", &self.to)
        .field("task", &self.task)
        .field("status", &self.status)
        .field("encrypted (#bytes)", &self.encrypted.len())
        .field("encryption_keys (#)", &self.encryption_keys.len())
        .field("metadata", &self.metadata)
        .finish()
    }
}