#![allow(unused_imports)]

use beam_id::{BeamId, AppId, AppOrProxyId};
use crypto_jwt::extract_jwt;
use errors::SamplyBeamError;
use serde_json::{Value, json};
use static_init::dynamic;
use tracing::debug;
use aes_gcm::{NewAead, aead::Aead, Aes256Gcm};
use rsa::{RsaPrivateKey, RsaPublicKey, PaddingScheme, PublicKey};

use std::{time::{Duration, Instant, SystemTime}, ops::Deref, fmt::Display};

use rand::Rng;
use serde::{Deserialize, Serialize, de::{Visitor, DeserializeOwned}};
use std::{collections::HashMap, str::FromStr};
use uuid::Uuid;

pub type MsgId = MyUuid;
pub type MsgType = String;
pub type TaskResponse = String;

mod traits;
pub mod logger;
pub mod crypto;
pub mod crypto_jwt;
pub mod errors;

pub mod config;
pub mod config_shared;
// #[cfg(feature = "config-for-broker")]
pub mod config_broker;
// #[cfg(feature = "config-for-proxy")]
pub mod config_proxy;

pub mod middleware;
pub mod http_proxy;
pub mod beam_id;

pub mod examples;

#[derive(Debug,Serialize,Deserialize,Clone,Copy,PartialEq,Eq,Hash)]
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

#[derive(PartialEq, Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase", tag = "status", content = "body")]
pub enum WorkStatus {
    Claimed,
    TempFailed(TaskResponse),
    PermFailed(TaskResponse),
    Succeeded(TaskResponse),
}

impl Display for WorkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            WorkStatus::Claimed => String::from("Claimed"),
            WorkStatus::TempFailed(e) => format!("Temporary failure: {e}"),
            WorkStatus::PermFailed(e) => format!("Permanent failure: {e}"),
            WorkStatus::Succeeded(e) => format!("Success: {e}"),
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

#[derive(Clone,Debug,Serialize,Deserialize, PartialEq)]
pub struct MsgSigned<M: Msg> {
    pub msg: M,
    pub sig: String
}

impl<M: Msg> MsgSigned<M> {
    pub async fn verify(&self) -> Result<(), SamplyBeamError> {
        // Signature valid?
        let (proxy_public_info, _, content) 
            = extract_jwt(&self.sig).await?;

        // Message content matches token?
        let val = serde_json::to_value(&self.msg)
            .expect("Internal error: Unable to interpret already parsed message to JSON Value.");
        if content.custom != val {
            return Err(SamplyBeamError::RequestValidationFailed("content.custom did not match parsed message.".to_string()));
        }

        // From field matches CN in certificate?
        if ! self.get_from().can_be_signed_by(&proxy_public_info.beam_id) {
            return Err(SamplyBeamError::RequestValidationFailed(format!("{} is not allowed to sign for {}", &proxy_public_info.beam_id, self.get_from())));
        }
        debug!("Message has been verified succesfully.");
        Ok(())
    }
}

#[dynamic]
pub static EMPTY_VEC_APPORPROXYID: Vec<AppOrProxyId> = Vec::new();

#[derive(Serialize,Deserialize,Debug)]
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

trait EncMsg<M>: Msg + Serialize where M: Msg + DeserializeOwned{
    /// Dectypts an encrypted message. Caution: can panic.
    fn decrypt(&self, my_id: &ClientId, my_priv_key: &RsaPrivateKey) -> Result<M,SamplyBrokerError> {
        // JSON parsing
        let mut encrypted_json = serde_json::to_value(&self).or_else(|_|Err(SamplyBrokerError::SignEncryptError("Decryption error: Cannot deserialize message.")))?
            .as_object().ok_or(SamplyBrokerError::SignEncryptError("Decryption error: Cannot deserialize message."))?;
        let encrypted = encrypted_json.remove("encrypted").ok_or(SamplyBrokerError::SignEncryptError("Decryption error: No encrypted payload found."))?
            .as_str().ok_or(SamplyBrokerError::SignEncryptError("Decryption error: Encrypted payload not readable."))?
            .as_bytes();
        let to_array_index: usize = encrypted_json.get("to").ok_or(SamplyBrokerError::SignEncryptError("Decryption error: 'to' field not readable."))?
            .as_array().ok_or(SamplyBrokerError::SignEncryptError("Decryption error: Cannot get adressee array."))?
            .iter()
            .position(|&entry| entry.as_str().expect("Decryption error: Cannot parse 'to' entries") == my_id.to_string()).ok_or(SamplyBrokerError::SignEncryptError("Decryption error: This client cannot be found in 'to' list."))?;
        let encrypted_decryption_key = encrypted_json.remove("encryption_keys").ok_or(SamplyBrokerError::SignEncryptError("Decryption error: Cannot read 'encryption_keys' field."))?
            .as_array().ok_or(SamplyBrokerError::SignEncryptError("Decryption error: Cannot read 'encrypted_keys' array."))?
            [to_array_index].as_str().ok_or(SamplyBrokerError::SignEncryptError("Decryption error: Encryption key is not readable."))?;
        // Cryptographic Operations
        let decryption_key = aes_gcm::Key::from_slice(&my_priv_key.decrypt(rsa::PaddingScheme::new_oaep::<sha2::Sha256>(), &encrypted_decryption_key.as_bytes())?);
        let nonce = aes_gcm::Nonce::from_slice(&encrypted[0..12]);
        let ciphertext = &encrypted[12..];
        let cipher = aes_gcm::Aes256Gcm::new(decryption_key);
        let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_|SamplyBrokerError::SignEncryptError("Decryption error: Cannot decrypt payload."))?;
        //JSON Reassembling
        let mut decrypted_json = encrypted_json;
        let decrypted_elements = serde_json::to_value(plaintext).map_err(|_|SamplyBrokerError::SignEncryptError("Decryption error: Decrypted plaintext invalid."))?
            .as_object().ok_or(SamplyBrokerError::SignEncryptError("Decryption error: Decrypted plaintext invalid."))?;
        for (key, value) in decrypted_elements {
            _ = decrypted_json.insert(*key, *value).ok_or(SamplyBrokerError::SignEncryptError("Decryption error: Cannot reassemble decrypted task."))?;
        }
        let result: M = serde_json::from_value(serde_json::Value::from(*decrypted_json)).or(Err(SamplyBrokerError::SignEncryptError("Decryption error: Cannot deserialize message")))?;
        Ok(result)
    }
}

trait DecMsg<M>: Msg + Serialize where M: Msg + DeserializeOwned {
    fn encrypt(&self, fields_to_encrypt: &Vec<&str>, reciever_public_keys: &Vec<RsaPublicKey>) -> Result<M, SamplyBrokerError> {
        let mut symmetric_key = [0;256];
        let mut nonce = [0;12];
        openssl::rand::rand_bytes(&mut symmetric_key).or_else(|_| Err(SamplyBrokerError::SignEncryptError("Encryption error: Cannot create symmetric key.")))?;
        openssl::rand::rand_bytes(&mut nonce).or_else(|_| Err(SamplyBrokerError::SignEncryptError("Encryption error: Cannot create nonce.")))?;

        let mut cleartext_json = serde_json::to_value(&self).or_else(|_|Err(SamplyBrokerError::SignEncryptError("Cannot deserialize message")))?
            .as_object().ok_or(SamplyBrokerError::SignEncryptError("Cannot deserialize message."))?;
        
        let mut rng = rand::thread_rng();
        let mut encrypted_keys = Vec::new();
        let encrypted_keys: Vec<String> = reciever_public_keys.iter()
            .encrypt(&mut rng, PaddingScheme::new_oaep(), &symmetric_key).or_else(|_| Err(SamplyBrokerError::SignEncryptError("Encryption error: Cannot encrypt symmetric key")))
            .collect();
        
        let mut json_to_encrypt = cleartext_json.clone();
        json_to_encrypt.retain(|k,_| fields_to_encrypt.contains(&k.as_str()));
        let mut encrypted_json = *cleartext_json;
        for f in fields_to_encrypt {
            _ = encrypted_json.remove(f);
        }

        encrypted_json.insert(String::from("encryption_keys"), serde_json::Value::from(encrypted_keys));

        let cipher = Aes256Gcm::new(aes_gcm::Key::from_slice(&symmetric_key));
        let plaintext = serde_json::Value::from(json_to_encrypt).as_str().ok_or(SamplyBrokerError::SignEncryptError("Encryption error: Cannot encrypt data."))?.as_bytes();
        let ciphertext = cipher.encrypt(aes_gcm::Nonce::from_slice(&nonce), plaintext).or(Err(SamplyBrokerError::SignEncryptError("Encryption error: Can not encrypt data.")))?;
        
        encrypted_json.insert(String::from("encrypted"), serde_json::Value::from(ciphertext));

        let result: M = serde_json::from_value(serde_json::Value::from(encrypted_json)).or(Err(SamplyBrokerError::SignEncryptError("Encryption error: Cannot deserialize message")))?;


        Ok(result)
    }

}

pub trait Msg: Serialize {
    fn get_from(&self) -> &AppOrProxyId;
    fn get_to(&self) -> &Vec<AppOrProxyId>;
    fn get_metadata(&self) -> &Value;
}

pub trait MsgWithBody : Msg{
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
    use std::{time::{SystemTime, UNIX_EPOCH, Duration}};

    use serde::{self, Deserialize, Deserializer, Serializer};
    use tracing::{warn, debug, error};


    pub fn serialize<S>(time: &SystemTime, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let ttl = match time.duration_since(SystemTime::now()) {
            Ok(v) => v,
            Err(e) => {
                error!("Internal Error: Tried to serialize a task which should have expired and expunged from memory {} seconds ago. Will return TTL=0. Cause: {}", e.duration().as_secs(), e);
                Duration::ZERO
            },
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
        Ok(
            expire
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MsgTaskRequest {
    pub id: MsgId,
    pub from: AppOrProxyId,
    pub to: Vec<AppOrProxyId>,
    pub body: String,
    #[serde(with="serialize_time", rename="ttl")]
    pub expire: SystemTime,
    pub failure_strategy: FailureStrategy,
    #[serde(skip)]
    pub results: HashMap<AppOrProxyId,MsgSigned<MsgTaskResult>>,
    pub metadata: Value,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncryptedMsgTaskRequest {
    pub id: MsgId,
    pub from: AppOrProxyId,
    pub to: Vec<AppOrProxyId>,
    //auth
    pub body: Option<String>,
    // pub expire: Instant,
    pub failure_strategy: Option<FailureStrategy>,
    pub encrypted: String,
    pub encryption_keys: Vec<Option<String>>,
    #[serde(skip)]
    pub results: HashMap<AppOrProxyId,MsgTaskResult>,
}

//TODO: Implement EncMsg and DecMsg for all message types
//impl<MsgTaskRequest> EncMsg<MsgTaskRequest> for EncryptedMsgTaskRequest{}
//impl<EncryptedMsgTaskRequest> DecMsg<EncryptedMsgTaskRequest> for MsgTaskRequest{}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MsgTaskResult {
    pub from: AppOrProxyId,
    pub to: Vec<AppOrProxyId>,
    pub task: MsgId,
    #[serde(flatten)]
    pub status: WorkStatus,
    pub metadata: Value,
}

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

impl<M> HasWaitId<MsgId> for MsgSigned<M> where M: HasWaitId<MsgId> + Msg {
    fn wait_id(&self) -> MsgId {
        self.msg.wait_id()
    }
}

impl<M> HasWaitId<String> for MsgSigned<M> where M: HasWaitId<String> + Msg {
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
        metadata: serde_json::Value
    ) -> Self {
        MsgTaskRequest {
            id: MsgId::new(),
            from,
            to,
            body,
            failure_strategy,
            results: HashMap::new(),
            metadata,
            expire: SystemTime::now() + Duration::from_secs(3600)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MsgPing {
    id: MsgId,
    from: AppOrProxyId,
    to: Vec<AppOrProxyId>,
    nonce: [u8; 16],
    metadata: Value
}

impl MsgPing {
    pub fn new(from: AppOrProxyId, to: AppOrProxyId) -> Self {
        let mut nonce = [0;16];
        openssl::rand::rand_bytes(&mut nonce)
            .expect("Critical Error: Failed to generate random byte array.");
        MsgPing { id: MsgId::new(), from, to: vec![to], nonce, metadata: json!(null) }
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