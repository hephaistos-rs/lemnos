//! SAML 2.0 service provider, behind the `saml` feature.
//!
//! Built on the pure-Rust `saml` crate, which is pre-1.0 and has had no
//! external security audit. To keep the attack surface small:
//!
//! - Only HTTP-Redirect requests and HTTP-POST responses are supported.
//! - Assertions must be signed; only strong algorithms are accepted.
//! - Encrypted assertions are not supported: pure-Rust RSA decryption is
//!   affected by RUSTSEC-2023-0071 (Marvin). TLS already protects the
//!   response in transit.
//! - Unsolicited (IdP-initiated) responses are refused.
//!
//! Assertion IDs are remembered in memory to stop replays, which is only
//! correct while Lemnos runs as a single instance.

use std::time::{Duration, SystemTime};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use saml::{
    Binding, ConsumeResponse, DigestAlgorithm, Dispatch, IdpDescriptor, InMemoryReplayCache,
    KeyPair, LoginTracker, NameIdFormat, PeerCryptoPolicy, PublicKeyAlgorithm, ReplayMode,
    ServiceProvider, ServiceProviderConfig, SignatureAlgorithm, SpWantSigned, SsoResponseBinding,
    SsoResponseEndpoint, StartLogin, X509Certificate,
};
use serde::Deserialize;
use url::Url;

use super::{Identity, LoginStart, PENDING_MAX_AGE, PendingFlow, PendingLogin, Secret, SsoError};

/// Clocks at the IdP and here may disagree by this much.
const CLOCK_SKEW: Duration = Duration::from_secs(60);

/// Attribute names IdPs commonly use, in order of preference.
const EMAIL_ATTRIBUTES: &[&str] = &[
    "email",
    "mail",
    "urn:oid:0.9.2342.19200300.100.1.3",
    "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress",
];
const NAME_ATTRIBUTES: &[&str] = &[
    "displayName",
    "name",
    "urn:oid:2.16.840.1.113730.3.1.241",
    "http://schemas.microsoft.com/identity/claims/displayname",
];
const USERNAME_ATTRIBUTES: &[&str] = &[
    "username",
    "uid",
    "urn:oid:0.9.2342.19200300.100.1.1",
    "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/name",
];

#[derive(Clone, Debug, Deserialize)]
pub struct SamlConfig {
    /// This service provider's entity ID, as registered at the IdP.
    pub entity_id: String,
    /// Assertion consumer service: where the IdP POSTs its response.
    pub acs_url: Url,
    pub idp_metadata: IdpMetadata,
    /// PKCS#8 PEM. When set, authentication requests are signed with it.
    #[serde(default)]
    pub signing_key_pem: Option<Secret>,
    /// PEM certificate for `signing_key_pem`, published in the metadata so
    /// the IdP can check those signatures.
    #[serde(default)]
    pub signing_cert_pem: Option<String>,
    /// Base64 of 32 random bytes. Seals the request tracker kept in the
    /// [`PendingLogin`]; anyone holding it can forge sign-in requests.
    pub tracker_key: Secret,
}

/// Where to get the IdP's metadata (its endpoints and signing certificates).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdpMetadata {
    /// Downloaded once at startup. Restart to pick up new certificates.
    Url(Url),
    /// The metadata XML itself.
    Xml(String),
}

/// The form the IdP POSTs to the assertion consumer service. Deserialize the
/// request body straight into this.
#[derive(Clone, Debug, Deserialize)]
pub struct SamlCallback {
    /// Base64, exactly as posted.
    #[serde(rename = "SAMLResponse")]
    pub saml_response: String,
    #[serde(rename = "RelayState", default)]
    pub relay_state: Option<String>,
}

pub(super) struct SamlProvider {
    sp: ServiceProvider,
    idp: IdpDescriptor,
    acs_url: Url,
    tracker_key: [u8; 32],
    replay_cache: InMemoryReplayCache,
}

impl SamlProvider {
    pub(super) async fn new(config: SamlConfig, http: &reqwest::Client) -> Result<Self, SsoError> {
        let tracker_key = BASE64
            .decode(config.tracker_key.expose().trim())
            .ok()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or_else(|| SsoError::Config("tracker_key must be base64 of 32 bytes".into()))?;

        let metadata = match &config.idp_metadata {
            IdpMetadata::Xml(xml) => xml.clone().into_bytes(),
            IdpMetadata::Url(url) => http
                .get(url.clone())
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|error| {
                    SsoError::Config(format!("downloading IdP metadata failed: {error}"))
                })?
                .bytes()
                .await
                .map_err(|error| {
                    SsoError::Config(format!("downloading IdP metadata failed: {error}"))
                })?
                .to_vec(),
        };
        let idp = IdpDescriptor::from_metadata_xml(&metadata)?;

        let signing_key = config
            .signing_key_pem
            .as_ref()
            .map(|pem| -> Result<KeyPair, SsoError> {
                let key = KeyPair::from_pkcs8_pem(pem.expose().as_bytes())?;
                Ok(match &config.signing_cert_pem {
                    Some(cert) => {
                        key.with_certificate(X509Certificate::from_pem(cert.as_bytes())?)?
                    }
                    None => key,
                })
            })
            .transpose()?;
        let outbound_signature_algorithm = match signing_key.as_ref().map(KeyPair::algorithm_family)
        {
            Some(PublicKeyAlgorithm::EcdsaP256) => SignatureAlgorithm::EcdsaSha256,
            Some(PublicKeyAlgorithm::EcdsaP384) => SignatureAlgorithm::EcdsaSha384,
            Some(PublicKeyAlgorithm::Rsa) | None => SignatureAlgorithm::RsaSha256,
        };

        let sp = ServiceProvider::new(ServiceProviderConfig {
            entity_id: config.entity_id,
            acs: vec![SsoResponseEndpoint::post(config.acs_url.as_str(), 0, true)],
            slo: vec![],
            name_id_formats: vec![NameIdFormat::Persistent, NameIdFormat::EmailAddress],
            sign_authn_requests: signing_key.is_some(),
            signing_key,
            decryption_key: None,
            want_signed: SpWantSigned {
                response: false,
                assertions: true,
            },
            allow_unsolicited: false,
            default_peer_crypto_policy: PeerCryptoPolicy::strong_defaults(),
            outbound_signature_algorithm,
            outbound_digest_algorithm: DigestAlgorithm::Sha256,
        })?;

        Ok(Self {
            sp,
            idp,
            acs_url: config.acs_url,
            tracker_key,
            replay_cache: InMemoryReplayCache::default(),
        })
    }

    pub(super) fn begin(&self, provider_id: &str) -> Result<LoginStart, SsoError> {
        let start = self.sp.start_login(
            &self.idp,
            StartLogin {
                relay_state: None,
                binding: Binding::HttpRedirect,
                force_authn: false,
                is_passive: false,
                requested_name_id_format: None,
                requested_authn_context: None,
                acs_index: None,
                acs_url: None,
                response_binding: None,
            },
        )?;
        let Dispatch::Redirect(redirect_to) = start.dispatch else {
            return Err(SsoError::Config(
                "the IdP has no HTTP-Redirect sign-on endpoint".into(),
            ));
        };
        Ok(LoginStart {
            redirect_to,
            pending: PendingLogin::new(
                provider_id,
                PendingFlow::Saml {
                    tracker: start.tracker.to_payload().seal(&self.tracker_key)?,
                },
            ),
        })
    }

    pub(super) fn finish(
        &self,
        provider_id: &str,
        tracker: &str,
        callback: &SamlCallback,
    ) -> Result<Identity, SsoError> {
        let now = SystemTime::now();
        let tracker = LoginTracker::open(tracker, &self.tracker_key, now, PENDING_MAX_AGE)?;
        // Some IdPs wrap the base64 over several lines.
        let encoded: Vec<u8> = callback
            .saml_response
            .bytes()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        let xml = BASE64
            .decode(encoded)
            .map_err(|_| SsoError::MissingParameter("SAMLResponse"))?;

        let identity = self.sp.consume_response(ConsumeResponse {
            idp: &self.idp,
            peer_crypto_policy: None,
            saml_response: &xml,
            binding: SsoResponseBinding::HttpPost,
            relay_state: callback.relay_state.as_deref(),
            tracker: Some(&tracker),
            expected_destination: self.acs_url.as_str(),
            now,
            clock_skew: CLOCK_SKEW,
            replay_cache: Some(&self.replay_cache),
            replay_mode: ReplayMode::All,
            holder_of_key_cert: None,
        })?;

        let name_id = identity.name_id();
        if name_id.format == NameIdFormat::Transient {
            return Err(SsoError::TransientSubject);
        }
        let attribute = |names: &[&str]| {
            names.iter().find_map(|name| {
                identity
                    .attributes()
                    .iter()
                    .find(|attribute| attribute.name == *name)
                    .and_then(|attribute| attribute.values.first())
                    .filter(|value| !value.is_empty())
                    .cloned()
            })
        };
        let email = attribute(EMAIL_ATTRIBUTES).or_else(|| {
            (name_id.format == NameIdFormat::EmailAddress).then(|| name_id.value.clone())
        });

        Ok(Identity {
            provider: provider_id.to_owned(),
            subject: name_id.value.clone(),
            email,
            email_verified: None,
            name: attribute(NAME_ATTRIBUTES),
            username: attribute(USERNAME_ATTRIBUTES),
        })
    }

    pub(super) fn metadata(&self) -> Result<String, SsoError> {
        Ok(self.sp.metadata_xml(false)?)
    }
}
