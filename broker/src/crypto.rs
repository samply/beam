use axum::async_trait;
use shared::{crypto::GetCerts, errors::SamplyBeamError, config};
use tracing::debug;
use vaultrs::{client::{VaultClientSettingsBuilderError, VaultClient, VaultClientSettingsBuilder}, error::ClientError, pki};

pub struct GetCertsFromPki {
    vault_client: VaultClient,
    pki_realm: String,
}

#[async_trait]
impl GetCerts for GetCertsFromPki {
    async fn certificate_list(&self) -> Result<Vec<String>,SamplyBeamError> {
        pki::cert::list(&self.vault_client, &self.pki_realm).await
            .map_err(|e| SamplyBeamError::VaultError(format!("Error in connection to certificate authority: {e}")))
    }

    async fn certificate_by_serial_as_pem(&self, serial: &str) -> Result<String,SamplyBeamError> {
        let certificate = pki::cert::read(&self.vault_client, &self.pki_realm, serial).await
            .map_err(|e| SamplyBeamError::VaultError(e.to_string()))?;
        Ok(certificate.certificate)
    }

    fn new() -> Result<Self,SamplyBeamError> {
        let mut certs = Vec::new();
        if let Some(dir) = &config::CONFIG_CENTRAL.tls_ca_certificates_dir {
            for file in std::fs::read_dir(dir).map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to read CA certificates: {}", e)))? {
                if let Ok(file) = file {
                    certs.push(file.path().to_str().unwrap().into());
                }
            }
            debug!("Loaded local certificates: {}", certs.join(" "));
        }
        let vault_client = VaultClient::new(
            VaultClientSettingsBuilder::default()
                .address(&config::CONFIG_CENTRAL.pki_address.to_string())
                .token(&config::CONFIG_CENTRAL.pki_token)
                .ca_certs(certs)
                .build()
                .map_err(|e| SamplyBeamError::VaultError(format!("Unable to build vault client settings: {}", e)))?
            ).map_err(|e| SamplyBeamError::VaultError(format!("Error in connection to certificate authority: {}", e)))?;
        
        let pki_realm = config::CONFIG_CENTRAL.pki_realm.clone();

        Ok(Self { vault_client, pki_realm })
    }
}

pub(crate) fn build_cert_getter() -> Result<GetCertsFromPki,SamplyBeamError> {
    GetCertsFromPki::new()
}