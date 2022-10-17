use crate::{SamplyBeamError, crypto::{self, load_certificates_from_dir, CryptoPublicPortion, GetCerts, get_all_certs_and_clients_by_cname_as_pemstr}, beam_id::{BrokerId, BeamId, ProxyId}, config::CONFIG_SHARED_CRYPTO};
use std::{path::PathBuf, rc::Rc, sync::Arc, fs::read_to_string};
use axum::async_trait;
use hyper::Uri;
use clap::Parser;
use hyper_tls::native_tls::Certificate;
use jwt_simple::prelude::RS256KeyPair;
use openssl::{x509::{X509, self}, asn1::Asn1IntegerRef};
use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey, pkcs1::DecodeRsaPrivateKey};
use static_init::dynamic;
use tracing::info;

pub(crate) const CLAP_FOOTER: &str = "For proxy support, environment variables HTTP_PROXY, HTTPS_PROXY, ALL_PROXY and NO_PROXY (and their lower-case variants) are supported. Usually, you want to set HTTP_PROXY *and* HTTPS_PROXY or set ALL_PROXY if both values are the same.\n\nFor updates and detailed usage instructions, visit https://github.com/samply/beam";

#[derive(Parser,Debug)]
#[clap(name("🌈 Samply.Beam (shared library)"), version, arg_required_else_help(true), after_help(crate::config_shared::CLAP_FOOTER))]
struct CliArgs {
    /// Outgoing HTTP proxy: Directory with CA certificates to trust for TLS connections (e.g. /etc/samply/cacerts/)
    #[clap(long, env, value_parser)]
    tls_ca_certificates_dir: Option<PathBuf>,

    /// samply.pki: Path to own secret key
    #[clap(long, env, value_parser, default_value = "/run/secrets/privkey.pem")]
    privkey_file: PathBuf,

    // TODO: The following arguments have been added for compatibility reasons with the proxy config. Find another way to merge configs.
    /// (included for technical reasons)
    #[clap(long, env, value_parser)]
    broker_url: Uri,

    /// (included for technical reasons)
    #[clap(long, env, value_parser)]
    proxy_id: Option<String>,

    /// (included for technical reasons)
    #[clap(action)]
    examples: Option<String>,

    /// (included for technical reasons)
    #[clap(long,hide(true))]
    test_threads: Option<String>
}

#[allow(dead_code)]
pub(crate) struct Config {
    pub(crate) tls_ca_certificates_dir: Option<PathBuf>,
    pub(crate) broker_domain: String,
}

pub(crate) struct ConfigCrypto {
    pub(crate) privkey_rs256: RS256KeyPair,
    pub(crate) privkey_rsa: RsaPrivateKey,
    pub(crate) public: CryptoPublicPortion
}

impl crate::config::Config for Config {
    fn load() -> Result<Self,SamplyBeamError> {
        let cli_args = CliArgs::parse();
        BrokerId::set_broker_id(&cli_args.broker_url.host().unwrap().to_string());
    
        let broker_domain = cli_args.broker_url.host();
        if false {
            todo!() // TODO Tobias: Check if matches certificate, and fail
        }
        let broker_domain = broker_domain.unwrap().to_string();
        let tls_ca_certificates_dir = cli_args.tls_ca_certificates_dir;
        Ok(Config { broker_domain, tls_ca_certificates_dir })
    }    
}

pub async fn init_crypto_for_proxy() -> Result<(String, String), SamplyBeamError>{
    let cli_args = CliArgs::parse();
    let crypto = load_crypto_for_proxy(&cli_args).await?;
    let serial = crypto.public.cert.serial_number().to_bn().unwrap().to_hex_str().unwrap().to_string();
    let cname = crypto.public.cert.subject_name().entries().next().unwrap().data().as_utf8()?.to_string();
    if CONFIG_SHARED_CRYPTO.set(crypto).is_err() {
        panic!("Tried to initialize crypto twice (init_crypto())");
    }
    Ok((serial, cname))
}

async fn load_crypto_for_proxy(cli_args: &CliArgs) -> Result<ConfigCrypto, SamplyBeamError> {
    let privkey_pem = read_to_string(&cli_args.privkey_file)
        .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to load private key from file {}: {}", cli_args.privkey_file.to_string_lossy(), e)))?
        .trim().to_string();
    let privkey_rsa = RsaPrivateKey::from_pkcs1_pem(&privkey_pem)
        .or_else(|_| RsaPrivateKey::from_pkcs8_pem(&privkey_pem))
        .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to interpret private key PEM as PKCS#1 or PKCS#8: {}", e)))?;
    let mut privkey_rs256 = RS256KeyPair::from_pem(&privkey_pem)
        .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to interpret private key PEM as PKCS#1 or PKCS#8: {}", e)))?;
    let proxy_id = cli_args.proxy_id.as_ref()
        .expect("load_crypto() has been called without setting a Proxy ID (maybe in broker?). This should not happen.");
    let proxy_id = ProxyId::new(proxy_id)?;
    let publics = get_all_certs_and_clients_by_cname_as_pemstr(&proxy_id).await
        .ok_or_else(|| SamplyBeamError::SignEncryptError("Unable to parse your certificate.".into()))?;
    let public = crypto::get_best_certificate(&publics, &privkey_rsa)
        .expect("Unable to choose valid, newest certificate for this proxy.");
    let serial = asn_str_to_vault_str(public.cert.serial_number())?;
    privkey_rs256 = privkey_rs256.with_key_id(&serial);
    let config = ConfigCrypto {
        privkey_rs256,
        privkey_rsa,
        public,
    };
    Ok(config)
}

fn asn_str_to_vault_str(asn: &Asn1IntegerRef) -> Result<String,SamplyBeamError> {
    let mut a = asn
        .to_bn()
        .map_err(|e| SamplyBeamError::SignEncryptError(format!("Unable to parse your certificate: {}", e)))?
        .to_hex_str()
        .map_err(|e| SamplyBeamError::SignEncryptError(format!("Unable to parse your certificate: {}", e)))?
        .to_string()
        .to_ascii_lowercase();

    let mut i=2;
    while i<a.len() {
        a.insert(i, ':');
        i+=3;
    }
    
    Ok(a)
}

#[cfg(test)]
mod test {
    use openssl::{asn1::{Asn1Integer, Asn1StringRef, Asn1String}, bn::BigNum};

    use super::asn_str_to_vault_str;


    #[test]
    fn hex_str() {
        let bn = BigNum::from_hex_str("440E0D94F36966391117BC9F867D84F0C48CFCB7").unwrap();
        let input = Asn1Integer::from_bn(&bn).unwrap();
        let expected = "44:0e:0d:94:f3:69:66:39:11:17:bc:9f:86:7d:84:f0:c4:8c:fc:b7";
        assert_eq!(expected, asn_str_to_vault_str(&input).unwrap());
    }
}
