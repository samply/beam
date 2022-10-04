use std::{net::AddrParseError, str::Utf8Error, string::FromUtf8Error};

use openssl::error::ErrorStack;

#[derive(thiserror::Error, Debug)]
pub enum SamplyBeamError {
    #[error("Invalid bind address supplied: {0}")]
    BindAddr(AddrParseError),
    #[error("Invalid broker address supplied: {0}")]
    WrongBrokerUri(&'static str),
    #[error("The request could not be validated.")]
    RequestValidationFailed(String),
    #[error("Invalid path supplied")]
    InvalidPath,
    #[error("Invalid Client ID: {0}")]
    InvalidClientIdString(String),
    #[error("Signing / encryption failed: {0}")]
    SignEncryptError(String),
    #[error("Communication with Samply.PKI failed: {0}")]
    VaultError(String),
    #[error("Unable to read config: {0}. Please check your environment and parameters.")]
    ConfigurationFailed(String),
    #[error("Internal synchronization error: {0}")]
    InternalSynchronizationError(String),
    #[error("Error executing HTTP request: {0}")]
    HttpRequestError(hyper::Error),
    #[error("Error building HTTP request: {0}")]
    HttpRequestBuildError(#[from] http::Error),
    #[error("Problem with HTTP proxy: {0}")]
    HttpProxyProblem(std::io::Error),
    #[error("Invalid Beam ID: {0}")]
    InvalidBeamId(String),
    #[error("Unable to parse HTTP response: {0}")]
    HttpParseError(FromUtf8Error)
}

impl From<AddrParseError> for SamplyBeamError {
    fn from(e: AddrParseError) -> Self {
        let ret = SamplyBeamError::BindAddr(e);
        println!("Building error: {}", ret);
        ret
    }
}

impl From<ErrorStack> for SamplyBeamError {
    fn from(e: ErrorStack) -> Self {
        Self::SignEncryptError(e.to_string())
    }
}

impl From<rsa::errors::Error> for SamplyBeamError {
    fn from(e: rsa::errors::Error) -> Self {
        Self::SignEncryptError(e.to_string())
    }
}

impl From<hyper::Error> for SamplyBeamError {
    fn from(e: hyper::Error) -> Self {
        Self::HttpRequestError(e)
    }
}