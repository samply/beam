use std::{net::AddrParseError};

use openssl::error::ErrorStack;
use thiserror::Error;
use vaultrs::{client::VaultClientSettingsBuilderError, error::ClientError};

#[derive(Error, Debug)]
pub enum SamplyBrokerError {
    #[error("Invalid bind address supplied: {0}")]
    BindAddr(AddrParseError),
    #[error("Invalid broker address supplied: {0}")]
    WrongBrokerUri(&'static str),
    #[error("The request could not be validated.")]
    ValidationFailed,
    #[error("Invalid path supplied")]
    InvalidPath,
    #[error("Signing / encryption failed")]
    SignEncryptError(&'static str),
    #[error("Communication with Samply.PKI failed")]
    VaultError(String),
    #[error("Unable to read secret config: {0}. Please check parameters --pki-secrets-file and --privkey_file.")]
    ReadSecretConfig(String),
    #[error("Internal synchronization error: {0}")]
    InternalSynchronizationError(String),
}

impl From<AddrParseError> for SamplyBrokerError {
    fn from(e: AddrParseError) -> Self {
        let ret = SamplyBrokerError::BindAddr(e);
        println!("Building error: {}", ret);
        ret
    }
}

impl From<VaultClientSettingsBuilderError> for SamplyBrokerError {
    fn from(_: VaultClientSettingsBuilderError) -> Self {
        Self::VaultError("Unable to build vault client settings.".into())
    }
}

impl From<ClientError> for SamplyBrokerError {
    fn from(e: ClientError) -> Self {
        Self::VaultError(format!("Error in connection to central certificate authority: {}", e))
    }
}

impl From<ErrorStack> for SamplyBrokerError {
    fn from(_: ErrorStack) -> Self {
        Self::SignEncryptError("SSL engine failed")
    }
}

impl From<std::io::Error> for SamplyBrokerError {
    fn from(e: std::io::Error) -> Self {
        Self::ReadSecretConfig(e.to_string())
    }
}

impl From<rsa::errors::Error> for SamplyBrokerError {
    fn from(e: rsa::errors::Error) -> Self {
        Self::SignEncryptError("Verification of signature failed")
    }
}