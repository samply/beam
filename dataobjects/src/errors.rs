#[derive(thiserror::Error, Debug)]
pub enum BeamIdError {
    #[error("Invalid Client ID: {0}")]
    InvalidClientIdString(String),
    #[error("Invalid Beam ID: {0}")]
    InvalidBeamId(String)
}


