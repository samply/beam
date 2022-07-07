use std::net::AddrParseError;

use openssl::error::ErrorStack;
use vaultrs::{client::VaultClientSettingsBuilderError, error::ClientError};

#[derive(thiserror::Error, Debug)]
pub enum SamplyBrokerError {
    #[error("Invalid bind address supplied: {0}")]
    BindAddr(AddrParseError),
    #[error("Invalid broker address supplied: {0}")]
    WrongBrokerUri(&'static str),
    #[error("The request could not be validated.")]
    ValidationFailed,
    #[error("Invalid path supplied")]
    InvalidPath,
    #[error("Signing / encryption failed: {0}")]
    SignEncryptError(String),
    #[error("Communication with Samply.PKI failed: {0}")]
    VaultError(String),
    #[error("Unable to read config: {0}. Please check your environment and parameters.")]
    ConfigurationFailed(String),
    #[error("Internal synchronization error: {0}")]
    InternalSynchronizationError(String)
}

impl From<AddrParseError> for SamplyBrokerError {
    fn from(e: AddrParseError) -> Self {
        let ret = SamplyBrokerError::BindAddr(e);
        println!("Building error: {}", ret);
        ret
    }
}

impl From<VaultClientSettingsBuilderError> for SamplyBrokerError {
    fn from(e: VaultClientSettingsBuilderError) -> Self {
        Self::VaultError(format!("Unable to build vault client settings: {}", e))
    }
}

impl From<ClientError> for SamplyBrokerError {
    fn from(e: ClientError) -> Self {
        Self::VaultError(format!("Error in connection to central certificate authority: {}", e))
    }
}

impl From<ErrorStack> for SamplyBrokerError {
    fn from(e: ErrorStack) -> Self {
        Self::SignEncryptError(e.to_string())
    }
}

impl From<std::io::Error> for SamplyBrokerError {
    fn from(e: std::io::Error) -> Self {
        Self::ConfigurationFailed(e.to_string())
    }
}

impl From<rsa::errors::Error> for SamplyBrokerError {
    fn from(e: rsa::errors::Error) -> Self {
        Self::SignEncryptError(e.to_string())
    }
}