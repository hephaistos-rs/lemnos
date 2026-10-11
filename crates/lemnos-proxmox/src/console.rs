//! Opening a guest's screen (its VNC console).
//!
//! Proxmox does the VNC part and a browser library such as noVNC draws it, so
//! nothing here understands the protocol. [`open`] only sets up the pipe:
//!
//! 1. It asks Proxmox to start a VNC server for the guest (`vncproxy`), which
//!    answers with a port and a one-time ticket.
//! 2. It connects a WebSocket to that port (`vncwebsocket`). Proxmox gives up
//!    on the VNC server after about ten seconds, so both happen in one call.
//!
//! What comes back is a [`Console`]: the open [`Socket`] carrying raw VNC
//! bytes, and the password the VNC client has to answer with. A browser can't
//! make this connection itself, as it has no way to send the login along, so
//! the server has to pass the bytes on in both directions.

use std::sync::Arc;

use proxmox_api::ReqwestError;
use proxmox_api::access::AccessClient;
use proxmox_api::access::ticket::PostParams as TicketParams;
use proxmox_api::nodes::NodesClient;
use proxmox_api::nodes::node::{lxc, qemu};
use reqwest::Url;
use rustls::ClientConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, COOKIE, SEC_WEBSOCKET_PROTOCOL};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use crate::client::{Client, Credentials, Endpoint};
use crate::guests::{GuestId, GuestKind};

/// One WebSocket message. VNC data travels as [`Message::Binary`].
pub use tokio_tungstenite::tungstenite::Message;

/// The connection to Proxmox that the VNC bytes travel over.
pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// An open console.
pub struct Console {
    /// The password the VNC client is asked for. With noVNC, pass it as
    /// `credentials: { password }`. It only works for this one connection.
    pub password: String,
    pub socket: Socket,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Proxmox rejected the request: {0}")]
    Api(#[from] ReqwestError),
    #[error("`{0}` is not a usable Proxmox address")]
    InvalidUrl(String),
    #[error("the login cannot be sent to Proxmox: {0}")]
    InvalidCredentials(&'static str),
    #[error("could not set up TLS: {0}")]
    Tls(#[from] rustls::Error),
    #[error("could not connect to the console: {0}")]
    WebSocket(#[from] Box<tokio_tungstenite::tungstenite::Error>),
}

/// Opens the console of `guest`.
///
/// `endpoint` and `credentials` have to be the ones `client` was connected
/// with: the WebSocket logs in separately, and the client doesn't give its
/// login back out.
pub async fn open(
    client: &Client,
    endpoint: &Endpoint,
    credentials: &Credentials,
    guest: &GuestId,
) -> Result<Console, Error> {
    let node = NodesClient::new(client).node(&guest.node);
    let (port, ticket) = match guest.kind {
        GuestKind::Qemu => {
            let params = qemu::vmid::vncproxy::PostParams {
                websocket: Some(true),
                ..Default::default()
            };
            let proxy = node.qemu().vmid(guest.vmid).vncproxy().post(params).await?;
            (proxy.port, proxy.ticket)
        }
        GuestKind::Lxc => {
            let params = lxc::vmid::vncproxy::PostParams {
                websocket: Some(true),
                ..Default::default()
            };
            let proxy = node.lxc().vmid(guest.vmid).vncproxy().post(params).await?;
            (proxy.port, proxy.ticket)
        }
    };

    let url = websocket_url(endpoint, guest, port, &ticket)?;
    let mut request = url.as_str().into_client_request().map_err(Box::new)?;
    let headers = request.headers_mut();
    let (name, login) = login_header(client, endpoint, credentials).await?;
    let login = HeaderValue::from_str(&login)
        .map_err(|_| Error::InvalidCredentials("it contains a character HTTP does not allow"))?;
    headers.insert(name, login);
    // Proxmox only speaks VNC over binary frames, and wants to be told so.
    headers.insert(SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static("binary"));

    let tls = Connector::Rustls(Arc::new(tls_config(endpoint.accept_invalid_certs)?));
    let (socket, _) =
        tokio_tungstenite::connect_async_tls_with_config(request, None, true, Some(tls))
            .await
            .map_err(Box::new)?;

    Ok(Console {
        password: ticket,
        socket,
    })
}

/// Builds the address of the guest's `vncwebsocket` endpoint.
fn websocket_url(
    endpoint: &Endpoint,
    guest: &GuestId,
    port: i64,
    ticket: &str,
) -> Result<Url, Error> {
    let invalid = || Error::InvalidUrl(endpoint.url.clone());

    let mut url = Url::parse(&endpoint.url).map_err(|_| invalid())?;
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        _ => return Err(invalid()),
    };
    url.set_scheme(scheme).map_err(|_| invalid())?;

    let kind = match guest.kind {
        GuestKind::Qemu => "qemu",
        GuestKind::Lxc => "lxc",
    };
    url.set_path(&format!(
        "/api2/json/nodes/{}/{kind}/{}/vncwebsocket",
        guest.node, guest.vmid
    ));
    // The ticket is full of `+`, `/` and `=`, which this escapes.
    url.query_pairs_mut()
        .append_pair("port", &port.to_string())
        .append_pair("vncticket", ticket);

    Ok(url)
}

/// The header that proves to Proxmox who is opening the WebSocket.
async fn login_header(
    client: &Client,
    endpoint: &Endpoint,
    credentials: &Credentials,
) -> Result<(reqwest::header::HeaderName, String), Error> {
    let user = format!("{}@{}", endpoint.user, endpoint.realm);

    match credentials {
        Credentials::ApiToken { id, secret } => {
            Ok((AUTHORIZATION, format!("PVEAPIToken={user}!{id}={secret}")))
        }
        // The client's own ticket can't be read back out of it, so log in
        // once more for a ticket that can be sent along.
        Credentials::Password(password) => {
            let user = user
                .try_into()
                .map_err(|_| Error::InvalidCredentials("the user name is too long"))?;
            let params = TicketParams::new(password.clone(), user);
            let login = AccessClient::new(client).ticket().post(params).await?;
            let ticket = login
                .ticket
                .ok_or(Error::InvalidCredentials("Proxmox returned no ticket"))?;
            Ok((COOKIE, format!("PVEAuthCookie={ticket}")))
        }
    }
}

/// TLS settings for the WebSocket, matching what the HTTP client does.
fn tls_config(accept_invalid_certs: bool) -> Result<ClientConfig, rustls::Error> {
    // Named explicitly: with more than one provider compiled into the final
    // program, rustls refuses to pick one by itself.
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?;

    Ok(if accept_invalid_certs {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate(provider)))
            .with_no_client_auth()
    } else {
        builder.with_platform_verifier()?.with_no_client_auth()
    })
}

/// Trusts whatever certificate the server shows, for
/// [`Endpoint::accept_invalid_certs`]. The handshake's signatures are still
/// checked, so the server does hold the key of the certificate it shows.
#[derive(Debug)]
struct AcceptAnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let algorithms = &self.0.signature_verification_algorithms;
        verify_tls12_signature(message, cert, dss, algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let algorithms = &self.0.signature_verification_algorithms;
        verify_tls13_signature(message, cert, dss, algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guests::VmId;

    fn guest(kind: GuestKind) -> GuestId {
        GuestId {
            node: "pve1".to_string(),
            vmid: VmId::try_from(100).unwrap(),
            kind,
        }
    }

    #[test]
    fn builds_the_websocket_address() {
        let endpoint = Endpoint::new("https://pve.example.com:8006/", "root", "pam");

        let url =
            websocket_url(&endpoint, &guest(GuestKind::Qemu), 5900, "PVEVNC:6A+b/c==").unwrap();

        assert_eq!(
            url.as_str(),
            "wss://pve.example.com:8006/api2/json/nodes/pve1/qemu/100/vncwebsocket\
             ?port=5900&vncticket=PVEVNC%3A6A%2Bb%2Fc%3D%3D"
        );
    }

    #[test]
    fn containers_and_plain_http_get_their_own_address() {
        let endpoint = Endpoint::new("http://10.0.0.5:8006", "root", "pam");

        let url = websocket_url(&endpoint, &guest(GuestKind::Lxc), 5901, "t").unwrap();

        assert!(
            url.as_str()
                .starts_with("ws://10.0.0.5:8006/api2/json/nodes/pve1/lxc/100/")
        );
    }

    #[test]
    fn rejects_an_address_that_is_not_http() {
        let endpoint = Endpoint::new("ftp://pve.example.com", "root", "pam");

        let error = websocket_url(&endpoint, &guest(GuestKind::Qemu), 5900, "t").unwrap_err();

        assert!(matches!(error, Error::InvalidUrl(_)));
    }

    #[test]
    fn builds_both_kinds_of_tls_settings() {
        tls_config(false).unwrap();
        tls_config(true).unwrap();
    }
}
