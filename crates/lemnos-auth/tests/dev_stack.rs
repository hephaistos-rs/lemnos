//! End-to-end sign-ins against the servers in `compose.dev.yml`.
//!
//! Ignored by default, since they need those containers running:
//!
//! ```text
//! docker compose -f compose.dev.yml up -d --wait
//! cargo test -p lemnos-auth --all-features -- --ignored
//! ```
//!
//! Where a real user would click through a login page in their browser,
//! these tests play the browser themselves with a plain HTTP client.

use lemnos_auth::{
    Auth, AuthConfig, AuthError, Authenticated, Identity, MemoryStore, UnknownIdentity,
    sso::{Callback, OAuthCallback, ProviderConfig, Sso, SsoError},
};
use reqwest::{Url, header::LOCATION, redirect::Policy};
use serde_json::json;

const NEEDS_STACK: &str = "run `docker compose -f compose.dev.yml up -d --wait` first";

/// Where Lemnos would be listening. Nothing has to run there: the tests stop
/// following redirects as soon as one points at this address.
const LEMNOS: &str = "http://localhost:3000";

fn sso_providers() -> Vec<ProviderConfig> {
    serde_json::from_value(json!([
        {
            "id": "dex",
            "display_name": "Dex (OIDC)",
            "type": "oidc",
            "issuer_url": "http://localhost:5556/dex",
            "client_id": "lemnos",
            "client_secret": "lemnos-dev-secret",
            "redirect_url": format!("{LEMNOS}/auth/sso/dex/callback"),
        },
        {
            "id": "dex-oauth",
            "display_name": "Dex (plain OAuth2)",
            "type": "oauth2",
            "client_id": "lemnos",
            "client_secret": "lemnos-dev-secret",
            "auth_url": "http://localhost:5556/dex/auth",
            "token_url": "http://localhost:5556/dex/token",
            "userinfo_url": "http://localhost:5556/dex/userinfo",
            "redirect_url": format!("{LEMNOS}/auth/sso/dex-oauth/callback"),
            "scopes": ["openid", "email", "profile"],
        },
    ]))
    .unwrap()
}

/// A browser stand-in: keeps cookies, but hands redirects back to the test
/// so it can see where each one goes.
fn browser() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(Policy::none())
        .cookie_store(true)
        .build()
        .unwrap()
}

/// Follows redirects from `url` until a page is shown (returns its URL and
/// HTML) or one leads back to Lemnos (returns that URL and no HTML).
async fn follow(
    browser: &reqwest::Client,
    mut response: reqwest::Response,
) -> (Url, Option<String>) {
    loop {
        let here = response.url().clone();
        if !response.status().is_redirection() {
            return (here, Some(response.text().await.unwrap()));
        }
        let location = response.headers()[LOCATION].to_str().unwrap();
        let next = here.join(location).unwrap();
        if next.as_str().starts_with(LEMNOS) {
            return (next, None);
        }
        response = browser.get(next).send().await.unwrap();
    }
}

/// Signs in on Dex's login page and returns what Dex sends back to Lemnos.
async fn sign_in_at_dex(start: Url, email: &str, password: &str) -> Result<OAuthCallback, String> {
    let browser = browser();
    let (login_page, _) = follow(&browser, browser.get(start).send().await.unwrap()).await;
    let submitted = browser
        .post(login_page)
        .form(&[("login", email), ("password", password)])
        .send()
        .await
        .unwrap();
    match follow(&browser, submitted).await {
        (callback, None) => Ok(callback_from(&callback)),
        // Dex shows the login page again when the password is wrong.
        (_, Some(page)) => Err(page),
    }
}

fn callback_from(url: &Url) -> OAuthCallback {
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    OAuthCallback {
        code: param("code"),
        state: param("state"),
        error: param("error"),
        error_description: param("error_description"),
    }
}

async fn sign_in_with(sso: &Sso, provider: &str) -> Identity {
    let start = sso.begin(provider).await.expect(NEEDS_STACK);
    let callback = sign_in_at_dex(start.redirect_to, "alice@lemnos.test", "password")
        .await
        .expect("Dex accepts alice's password");
    sso.finish(provider, start.pending, Callback::OAuth(callback))
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "needs the compose.dev.yml stack"]
async fn oidc_sign_in() {
    let sso = Sso::from_config(sso_providers()).await.expect(NEEDS_STACK);
    let identity = sign_in_with(&sso, "dex").await;

    assert_eq!(identity.provider, "dex");
    assert_eq!(identity.email.as_deref(), Some("alice@lemnos.test"));
    assert_eq!(identity.email_verified, Some(true));
    assert!(!identity.subject.is_empty());

    // The subject is stable: a second sign-in is the same person.
    assert_eq!(sign_in_with(&sso, "dex").await.subject, identity.subject);

    // And it maps to one Lemnos account.
    let auth = Auth::new(MemoryStore::new(), AuthConfig::default());
    let first = auth
        .authenticate_identity(&identity, UnknownIdentity::CreateAccount)
        .await
        .unwrap();
    let second = auth
        .authenticate_identity(&identity, UnknownIdentity::Reject)
        .await
        .unwrap();
    assert_eq!(first.user().id, second.user().id);
}

#[tokio::test]
#[ignore = "needs the compose.dev.yml stack"]
async fn oidc_rejects_tampering() {
    let sso = Sso::from_config(sso_providers()).await.expect(NEEDS_STACK);

    // A wrong password never produces a callback at all.
    let start = sso.begin("dex").await.unwrap();
    assert!(
        sign_in_at_dex(start.redirect_to, "alice@lemnos.test", "wrong")
            .await
            .is_err()
    );

    // A callback whose `state` was changed is refused before anything else.
    let start = sso.begin("dex").await.unwrap();
    let mut callback = sign_in_at_dex(start.redirect_to, "alice@lemnos.test", "password")
        .await
        .unwrap();
    callback.state = Some("forged".into());
    assert!(matches!(
        sso.finish("dex", start.pending, Callback::OAuth(callback))
            .await,
        Err(SsoError::StateMismatch)
    ));

    // A code can't be redeemed with another sign-in's PKCE verifier: here
    // alice's code is paired with the pending state of a second sign-in.
    let first = sso.begin("dex").await.unwrap();
    let second = sso.begin("dex").await.unwrap();
    let stolen = sign_in_at_dex(first.redirect_to, "alice@lemnos.test", "password")
        .await
        .unwrap();
    let own = sign_in_at_dex(second.redirect_to, "bob@lemnos.test", "password")
        .await
        .unwrap();
    let mixed = OAuthCallback {
        code: stolen.code,
        ..own
    };
    assert!(matches!(
        sso.finish("dex", second.pending, Callback::OAuth(mixed))
            .await,
        Err(SsoError::TokenExchange(_))
    ));
}

#[tokio::test]
#[ignore = "needs the compose.dev.yml stack"]
async fn generic_oauth2_sign_in() {
    let sso = Sso::from_config(sso_providers()).await.expect(NEEDS_STACK);
    let via_oauth = sign_in_with(&sso, "dex-oauth").await;

    assert_eq!(via_oauth.provider, "dex-oauth");
    assert_eq!(via_oauth.email.as_deref(), Some("alice@lemnos.test"));

    // Same user at the same server, so the userinfo lookup must arrive at
    // the subject the signed ID token carries.
    let via_oidc = sign_in_with(&sso, "dex").await;
    assert_eq!(via_oauth.subject, via_oidc.subject);
}

#[cfg(feature = "ldap")]
mod ldap {
    use lemnos_auth::{Login, Method, ldap::LdapConfig};

    use super::*;

    fn new_auth() -> Auth<MemoryStore> {
        let config: LdapConfig = serde_json::from_value(json!({
            "url": "ldap://localhost:3890",
            // The test directory has no TLS; it only listens on localhost.
            "danger_allow_plaintext": true,
            "bind_dn": "uid=admin,ou=people,dc=lemnos,dc=test",
            "bind_password": "admin-password",
            "user_base_dn": "ou=people,dc=lemnos,dc=test",
        }))
        .unwrap();
        Auth::new(MemoryStore::new(), AuthConfig::default())
            .with_ldap(config)
            .unwrap()
    }

    async fn sign_in(auth: &Auth<MemoryStore>, username: &str, password: &str) -> Authenticated {
        match auth
            .login_ldap(username, password, UnknownIdentity::CreateAccount)
            .await
            .expect(NEEDS_STACK)
        {
            Login::Complete(authenticated) => authenticated,
            Login::SecondFactor(_) => panic!("no second factor is set up"),
        }
    }

    #[tokio::test]
    #[ignore = "needs the compose.dev.yml stack"]
    async fn ldap_sign_in() {
        let auth = new_auth();
        let alice = sign_in(&auth, "alice", "alice-password").await;
        assert_eq!(alice.method(), Method::Ldap);
        assert_eq!(alice.user().username.as_str(), "alice");
        assert_eq!(alice.user().name, "Alice Liddell");
        assert_eq!(alice.user().email.as_deref(), Some("alice@lemnos.test"));

        // Signing in again finds the account made the first time.
        let again = sign_in(&auth, "Alice", "alice-password").await;
        assert_eq!(again.user().id, alice.user().id);

        let bob = sign_in(&auth, "bob", "bob-password").await;
        assert_ne!(bob.user().id, alice.user().id);
    }

    #[tokio::test]
    #[ignore = "needs the compose.dev.yml stack"]
    async fn ldap_rejects_bad_sign_ins() {
        let auth = new_auth();
        // Make sure the directory is reachable, so the failures below are
        // real rejections and not connection errors.
        sign_in(&auth, "bob", "bob-password").await;

        for (username, password) in [
            ("alice", "wrong-password"),
            ("alice", ""),
            ("nobody", "alice-password"),
            // Filter injection: would match every user if not escaped.
            ("*", "alice-password"),
            ("alice)(uid=*", "alice-password"),
        ] {
            let result = auth
                .login_ldap(username, password, UnknownIdentity::CreateAccount)
                .await;
            assert!(
                matches!(result, Err(AuthError::InvalidCredentials)),
                "{username:?} / {password:?} gave {result:?}"
            );
        }

        // Without permission to create accounts, a directory user who has
        // never signed in before is turned away.
        let strict = new_auth();
        assert!(matches!(
            strict
                .login_ldap("alice", "alice-password", UnknownIdentity::Reject)
                .await,
            Err(AuthError::NoLinkedAccount)
        ));
    }
}

#[cfg(feature = "saml")]
mod saml {
    use lemnos_auth::sso::SamlCallback;

    use super::*;

    fn providers() -> Vec<ProviderConfig> {
        serde_json::from_value(json!([{
            "id": "saml",
            "display_name": "SimpleSAMLphp",
            "type": "saml",
            "entity_id": format!("{LEMNOS}/auth/sso/saml"),
            "acs_url": format!("{LEMNOS}/auth/sso/saml/acs"),
            "idp_metadata": { "url": "http://localhost:8088/simplesaml/saml2/idp/metadata.php" },
            // Base64 of 32 bytes; any value works for a test.
            "tracker_key": "bGVtbm9zLWRldi10cmFja2VyLWtleS0zMi1ieXRlcyE=",
        }]))
        .unwrap()
    }

    /// The value of the hidden `<input name="...">` in an HTML form.
    fn hidden_input(html: &str, name: &str) -> Option<String> {
        let tag = html
            .split("<input")
            .find(|tag| tag.contains(&format!("name=\"{name}\"")))?;
        let value = tag.split("value=\"").nth(1)?.split('"').next()?;
        Some(
            value
                .replace("&amp;", "&")
                .replace("&#x3D;", "=")
                .replace("&#x2B;", "+"),
        )
    }

    /// Signs in on the IdP's login page and returns the form it would have
    /// the browser POST back to Lemnos.
    async fn sign_in_at_idp(
        start: Url,
        username: &str,
        password: &str,
    ) -> Result<SamlCallback, String> {
        let browser = browser();
        let (login_page, html) = follow(&browser, browser.get(start).send().await.unwrap()).await;
        let html = html.expect("the IdP shows a login page");
        let auth_state = hidden_input(&html, "AuthState").expect("login form has AuthState");
        let submitted = browser
            .post(login_page)
            .form(&[
                ("username", username),
                ("password", password),
                ("AuthState", &auth_state),
            ])
            .send()
            .await
            .unwrap();
        let (_, page) = follow(&browser, submitted).await;
        let page = page.expect("the IdP answers with a page");
        match hidden_input(&page, "SAMLResponse") {
            Some(saml_response) => Ok(SamlCallback {
                saml_response,
                relay_state: hidden_input(&page, "RelayState"),
            }),
            None => Err(page),
        }
    }

    #[tokio::test]
    #[ignore = "needs the compose.dev.yml stack"]
    async fn saml_sign_in() {
        let sso = Sso::from_config(providers()).await.expect(NEEDS_STACK);
        assert!(
            sso.saml_metadata("saml")
                .unwrap()
                .contains("/auth/sso/saml/acs")
        );

        let start = sso.begin("saml").await.unwrap();
        let callback = sign_in_at_idp(start.redirect_to, "alice", "alice-password")
            .await
            .expect("the IdP accepts alice's password");
        let identity = sso
            .finish("saml", start.pending, Callback::Saml(callback.clone()))
            .await
            .unwrap();
        assert_eq!(identity.provider, "saml");
        assert_eq!(identity.subject, "alice");
        assert_eq!(identity.email.as_deref(), Some("alice@lemnos.test"));
        assert_eq!(identity.name.as_deref(), Some("Alice Liddell"));

        // The same response a second time is a replay, even with a fresh
        // sign-in request to pair it with.
        let again = sso.begin("saml").await.unwrap();
        assert!(
            sso.finish("saml", again.pending, Callback::Saml(callback))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    #[ignore = "needs the compose.dev.yml stack"]
    async fn saml_rejects_tampering() {
        let sso = Sso::from_config(providers()).await.expect(NEEDS_STACK);

        let start = sso.begin("saml").await.unwrap();
        assert!(
            sign_in_at_idp(start.redirect_to, "alice", "wrong")
                .await
                .is_err()
        );

        // Bob signs in, then edits the response to say he is alice. The
        // signature no longer matches, so it must be refused.
        let start = sso.begin("saml").await.unwrap();
        let mut callback = sign_in_at_idp(start.redirect_to, "bob", "bob-password")
            .await
            .unwrap();
        callback.saml_response = reencode(&callback.saml_response, |xml| {
            xml.replace(">bob<", ">alice<")
        });
        assert!(matches!(
            sso.finish("saml", start.pending, Callback::Saml(callback))
                .await,
            Err(SsoError::Saml(_))
        ));

        // A valid response for one sign-in can't complete a different one.
        let first = sso.begin("saml").await.unwrap();
        let second = sso.begin("saml").await.unwrap();
        let callback = sign_in_at_idp(first.redirect_to, "bob", "bob-password")
            .await
            .unwrap();
        assert!(matches!(
            sso.finish("saml", second.pending, Callback::Saml(callback))
                .await,
            Err(SsoError::Saml(_))
        ));
    }

    fn reencode(base64_xml: &str, edit: impl FnOnce(String) -> String) -> String {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let xml = String::from_utf8(STANDARD.decode(base64_xml).unwrap()).unwrap();
        let edited = edit(xml.clone());
        assert_ne!(edited, xml, "the edit should change something");
        STANDARD.encode(edited)
    }
}
