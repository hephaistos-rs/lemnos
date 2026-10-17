//! Reading an API token ID the way people enter it.
//!
//! Proxmox shows a token as `user@realm!name`, e.g. `root@pam!lemnos`, and
//! that is what gets copied. [`Credentials::ApiToken`] wants only the name,
//! with the user and realm on the [`Endpoint`]. [`split`] takes either form,
//! next to separately given user and realm, and returns the three parts.
//!
//! [`Credentials::ApiToken`]: crate::client::Credentials::ApiToken
//! [`Endpoint`]: crate::client::Endpoint

/// Who a token belongs to, and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenOwner {
    pub user: String,
    pub realm: String,
    /// The part after `!`.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenIdError {
    #[error("the token ID is empty")]
    Empty,
    #[error("the token ID has more than one `!`; it looks like root@pam!lemnos")]
    ExtraSeparator,
    #[error("the token ID has nothing after `!`; add the token's name, as in root@pam!lemnos")]
    MissingName,
    #[error("the token ID has no user before `@`; it looks like root@pam!lemnos")]
    MissingUser,
    #[error("the token ID has no realm between `@` and `!`; it looks like root@pam!lemnos")]
    MissingRealm,
    #[error("`{0}` is a user, not a token; add `!` and the token's name, as in root@pam!lemnos")]
    UserOnly(String),
    #[error("a token name starts with a letter and holds only letters, digits, `.`, `-` and `_`")]
    BadName,
    #[error("the token ID is for user `{token}`, but the user is set to `{given}`")]
    UserConflict { token: String, given: String },
    #[error("the token ID is for realm `{token}`, but the realm is set to `{given}`")]
    RealmConflict { token: String, given: String },
}

/// Splits `token_id` into user, realm and name.
///
/// `token_id` is either just the name (`lemnos`), which then needs `user`
/// and `realm`, or the full `user@realm!name`, which then needs neither. An
/// empty `user` or `realm` counts as not given. When both are there, they
/// have to agree: which one does not is the error.
pub fn split(token_id: &str, user: &str, realm: &str) -> Result<TokenOwner, TokenIdError> {
    let token_id = token_id.trim();
    let (user, realm) = (user.trim(), realm.trim());
    if token_id.is_empty() {
        return Err(TokenIdError::Empty);
    }

    let Some((owner, name)) = token_id.split_once('!') else {
        if token_id.contains('@') {
            return Err(TokenIdError::UserOnly(token_id.to_owned()));
        }
        check_name(token_id)?;
        if user.is_empty() {
            return Err(TokenIdError::MissingUser);
        }
        if realm.is_empty() {
            return Err(TokenIdError::MissingRealm);
        }
        return Ok(TokenOwner {
            user: user.to_owned(),
            realm: realm.to_owned(),
            name: token_id.to_owned(),
        });
    };

    if name.contains('!') {
        return Err(TokenIdError::ExtraSeparator);
    }
    if name.is_empty() {
        return Err(TokenIdError::MissingName);
    }
    check_name(name)?;
    // A user name may hold `@` itself (`jo@example.com@pve`); the realm
    // never does, so it is whatever follows the last one.
    let Some((token_user, token_realm)) = owner.rsplit_once('@') else {
        return Err(TokenIdError::MissingRealm);
    };
    if token_user.is_empty() {
        return Err(TokenIdError::MissingUser);
    }
    if token_realm.is_empty() {
        return Err(TokenIdError::MissingRealm);
    }
    if !user.is_empty() && user != token_user {
        return Err(TokenIdError::UserConflict {
            token: token_user.to_owned(),
            given: user.to_owned(),
        });
    }
    if !realm.is_empty() && realm != token_realm {
        return Err(TokenIdError::RealmConflict {
            token: token_realm.to_owned(),
            given: realm.to_owned(),
        });
    }
    Ok(TokenOwner {
        user: token_user.to_owned(),
        realm: token_realm.to_owned(),
        name: name.to_owned(),
    })
}

/// Proxmox's own rule for token names.
fn check_name(name: &str) -> Result<(), TokenIdError> {
    let mut chars = name.chars();
    let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest_allowed = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if starts_with_letter && rest_allowed {
        Ok(())
    } else {
        Err(TokenIdError::BadName)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(user: &str, realm: &str, name: &str) -> TokenOwner {
        TokenOwner {
            user: user.into(),
            realm: realm.into(),
            name: name.into(),
        }
    }

    #[test]
    fn the_name_alone_takes_user_and_realm_from_beside_it() {
        assert_eq!(
            split("lemnos", "root", "pam"),
            Ok(owner("root", "pam", "lemnos"))
        );
        assert_eq!(split("lemnos", "", "pam"), Err(TokenIdError::MissingUser));
        assert_eq!(split("lemnos", "root", ""), Err(TokenIdError::MissingRealm));
    }

    #[test]
    fn the_full_form_needs_nothing_beside_it() {
        assert_eq!(
            split("root@pam!lemnos", "", ""),
            Ok(owner("root", "pam", "lemnos"))
        );
        // Matching fields are fine, and surrounding spaces from a paste too.
        assert_eq!(
            split(" root@pam!lemnos ", "root", "pam"),
            Ok(owner("root", "pam", "lemnos"))
        );
        assert_eq!(
            split("jo@example.com@pve!ci", "", ""),
            Ok(owner("jo@example.com", "pve", "ci"))
        );
    }

    #[test]
    fn missing_parts_are_named() {
        assert_eq!(split("", "root", "pam"), Err(TokenIdError::Empty));
        assert_eq!(split("root@pam!", "", ""), Err(TokenIdError::MissingName));
        assert_eq!(split("@pam!lemnos", "", ""), Err(TokenIdError::MissingUser));
        assert_eq!(
            split("root@!lemnos", "", ""),
            Err(TokenIdError::MissingRealm)
        );
        assert_eq!(
            split("root!lemnos", "", ""),
            Err(TokenIdError::MissingRealm)
        );
        assert_eq!(
            split("root@pam", "", ""),
            Err(TokenIdError::UserOnly("root@pam".into()))
        );
    }

    #[test]
    fn conflicts_say_which_part() {
        assert_eq!(
            split("root@pam!lemnos", "root", "pve"),
            Err(TokenIdError::RealmConflict {
                token: "pam".into(),
                given: "pve".into()
            })
        );
        assert_eq!(
            split("root@pam!lemnos", "admin", ""),
            Err(TokenIdError::UserConflict {
                token: "root".into(),
                given: "admin".into()
            })
        );
    }

    #[test]
    fn an_extra_separator_or_odd_name_is_refused() {
        assert_eq!(
            split("root@pam!lemnos!x", "", ""),
            Err(TokenIdError::ExtraSeparator)
        );
        assert_eq!(
            split("root@pam!!lemnos", "", ""),
            Err(TokenIdError::ExtraSeparator)
        );
        assert_eq!(split("1token", "root", "pam"), Err(TokenIdError::BadName));
        assert_eq!(
            split("root@pam!my token", "", ""),
            Err(TokenIdError::BadName)
        );
    }
}
