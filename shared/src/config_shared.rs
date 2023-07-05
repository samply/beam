use beam_lib::ProxyId;
use crate::{
    config::CONFIG_SHARED_CRYPTO,
    crypto::{
        self, get_all_certs_and_clients_by_cname_as_pemstr, load_certificates_from_dir,
        CryptoPublicPortion, GetCerts,
    },
    SamplyBeamError,
};
use axum::async_trait;
use clap::Parser;
use hyper::Uri;
use hyper_tls::native_tls::Certificate;
use jwt_simple::prelude::RS256KeyPair;
use openssl::{
    asn1::Asn1IntegerRef,
    x509::{self, X509},
};
use rsa::{pkcs1::DecodeRsaPrivateKey, pkcs8::DecodePrivateKey, RsaPrivateKey};
use static_init::dynamic;
use std::{fs::read_to_string, path::PathBuf, rc::Rc, sync::Arc};
use tracing::{debug, info};

pub(crate) const CLAP_FOOTER: &str = "For proxy support, environment variables HTTP_PROXY, HTTPS_PROXY, ALL_PROXY and NO_PROXY (and their lower-case variants) are supported. Usually, you want to set HTTP_PROXY *and* HTTPS_PROXY or set ALL_PROXY if both values are the same.\n\nFor updates and detailed usage instructions, visit https://github.com/samply/beam";

#[derive(Parser, Debug)]
#[clap(
    name("🌈 Samply.Beam (shared library)"),
    version,
    arg_required_else_help(true),
    after_help(crate::config_shared::CLAP_FOOTER)
)]
struct CliArgs {
    /// Outgoing HTTP proxy: Directory with CA certificates to trust for TLS connections (e.g. /etc/samply/cacerts/)
    #[clap(long, env, value_parser)]
    tls_ca_certificates_dir: Option<PathBuf>,

    /// samply.pki: Path to own secret key
    #[clap(long, env, value_parser, default_value = "/run/secrets/privkey.pem")]
    privkey_file: PathBuf,

    /// samply.pki: Path to CA Root certificate
    #[clap(long, env, value_parser, default_value = "/run/secrets/root.crt.pem")]
    rootcert_file: PathBuf,

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
    #[clap(long, hide(true))]
    test_threads: Option<String>,
}

#[allow(dead_code)]
pub struct Config {
    pub(crate) tls_ca_certificates_dir: Option<PathBuf>,
    pub broker_domain: String,
    pub root_cert: X509,
    pub tls_ca_certificates: Vec<X509>,
}

#[derive(Debug, Clone)]
pub struct ConfigCrypto {
    pub privkey_rs256: RS256KeyPair,
    pub privkey_rsa: RsaPrivateKey,
    pub public: Option<CryptoPublicPortion>,
}

impl crate::config::Config for Config {
    fn load() -> Result<Self, SamplyBeamError> {
        let cli_args = CliArgs::parse();
        beam_lib::set_broker_id(cli_args.broker_url.host().unwrap().to_string());

        let root_cert = crypto::load_certificates_from_file(cli_args.rootcert_file)?;
        let broker_domain = cli_args.broker_url.host();
        if false {
            todo!() // TODO Tobias: Check if matches certificate, and fail
        }
        let broker_domain = broker_domain.unwrap().to_string();
        let tls_ca_certificates_dir = cli_args.tls_ca_certificates_dir;
        let tls_ca_certificates = crate::crypto::load_certificates_from_dir(
            tls_ca_certificates_dir.clone(),
        )
        .map_err(|e| {
            SamplyBeamError::ConfigurationFailed(format!(
                "Unable to read from TLS CA directory: {}",
                e
            ))
        })?;
        Ok(Config {
            broker_domain,
            tls_ca_certificates_dir,
            root_cert,
            tls_ca_certificates,
        })
    }
}

fn get_enrollment_msg(proxy_id: &Option<String>) -> String {
    let divider = "***************************************************************************\n
                   ***              Beam Certificate Enrollment Warning                    ***\n
                   ***************************************************************************";
    format!(
        "{}\nIf you are not yet enrolled in the central certificate store, please execute the beam-enroll companion tool (https://github.com/samply/beam-enroll) by executing:\n  docker run --rm -it -v \"$(pwd)\":/data -e PROXY_ID={} samply/beam-enroll\nand follow the steps on the screen.\nAfter your certificate signing request (CSR) has been approved, please restart this Beam.Proxy and this message should disappear.",
        divider,
        proxy_id.as_deref().unwrap_or("<proxy_id>")
    )
}

pub async fn init_public_crypto_for_proxy(
    private_config: ConfigCrypto,
) -> Result<(String, String), SamplyBeamError> {
    let cli_args = CliArgs::parse();
    let crypto = load_public_crypto_for_proxy(&cli_args, private_config).await?;

    let cert_info = crypto::ProxyCertInfo::try_from(
        &crypto
            .public
            .as_ref()
            .expect("Should be set by load_public_crypto_for_proxy")
            .cert,
    )?;
    if CONFIG_SHARED_CRYPTO.set(crypto).is_err() {
        panic!("Tried to initialize crypto twice (init_public_crypto_for_proxy())");
    }
    Ok((cert_info.serial, cert_info.common_name))
}

pub fn load_private_crypto_for_proxy() -> Result<ConfigCrypto, SamplyBeamError> {
    let cli_args = CliArgs::parse();
    let privkey_pem = read_to_string(&cli_args.privkey_file)
        .map_err(|e| {
            SamplyBeamError::ConfigurationFailed(format!(
                "Unable to load private key from file {}: {}\n{}",
                cli_args.privkey_file.to_string_lossy(),
                e,
                get_enrollment_msg(&cli_args.proxy_id)
            ))
        })?
        .trim()
        .to_string();
    let privkey_rsa = RsaPrivateKey::from_pkcs1_pem(&privkey_pem)
        .or_else(|_| RsaPrivateKey::from_pkcs8_pem(&privkey_pem))
        .map_err(|e| {
            SamplyBeamError::ConfigurationFailed(format!(
                "Unable to interpret private key PEM as PKCS#1 or PKCS#8: {}",
                e
            ))
        })?;
    let privkey_rs256 = RS256KeyPair::from_pem(&privkey_pem).map_err(|e| {
        SamplyBeamError::ConfigurationFailed(format!(
            "Unable to interpret private key PEM as PKCS#1 or PKCS#8: {}",
            e
        ))
    })?;
    Ok(ConfigCrypto {
        privkey_rs256,
        privkey_rsa,
        public: None,
    })
}

async fn load_public_crypto_for_proxy(
    cli_args: &CliArgs,
    mut config: ConfigCrypto,
) -> Result<ConfigCrypto, SamplyBeamError> {
    let proxy_id = cli_args.proxy_id.as_ref()
        .expect("load_crypto() has been called without setting a Proxy ID (maybe in broker?). This should not happen.");
    let proxy_id = ProxyId::new(proxy_id)?;
    let publics: Vec<CryptoPublicPortion> = get_all_certs_and_clients_by_cname_as_pemstr(&proxy_id)
        .await
        .into_iter()
        .filter_map(|r| {
            r.map_err(|e| debug!("Unable to parse Certificate: {e}"))
                .ok()
        })
        .collect();
    let public = crypto::get_best_own_certificate(publics, &config.privkey_rsa).ok_or(
        SamplyBeamError::SignEncryptError(
            "Unable to choose valid, newest certificate for this proxy".into(),
        ),
    )?;
    let serial = asn_str_to_vault_str(public.cert.serial_number())?;
    config.privkey_rs256 = config.privkey_rs256.with_key_id(&serial);
    config.public = Some(public);
    Ok(config)
}

fn asn_str_to_vault_str(asn: &Asn1IntegerRef) -> Result<String, SamplyBeamError> {
    let mut a = asn
        .to_bn()
        .map_err(|e| {
            SamplyBeamError::SignEncryptError(format!("Unable to parse your certificate: {}", e))
        })?
        .to_hex_str()
        .map_err(|e| {
            SamplyBeamError::SignEncryptError(format!("Unable to parse your certificate: {}", e))
        })?
        .to_string()
        .to_ascii_lowercase();

    let mut i = 2;
    while i < a.len() {
        a.insert(i, ':');
        i += 3;
    }

    Ok(a)
}

#[cfg(test)]
mod test {
    use openssl::{
        asn1::{Asn1Integer, Asn1String, Asn1StringRef},
        bn::BigNum,
    };

    use super::asn_str_to_vault_str;

    #[test]
    fn hex_str() {
        let bn = BigNum::from_hex_str("440E0D94F36966391117BC9F867D84F0C48CFCB7").unwrap();
        let input = Asn1Integer::from_bn(&bn).unwrap();
        let expected = "44:0e:0d:94:f3:69:66:39:11:17:bc:9f:86:7d:84:f0:c4:8c:fc:b7";
        assert_eq!(expected, asn_str_to_vault_str(&input).unwrap());
    }
}
