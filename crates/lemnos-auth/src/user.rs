//! Who a Lemnos account is.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// A user's permanent ID, handed out by the [`Store`](crate::Store).
///
/// A "newtype": a `u64` wrapped in its own type. It costs nothing at runtime,
/// but the compiler now refuses to mix a `UserId` up with any other number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UserId(pub u64);

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A sign-in name that has already been checked and normalised.
///
/// The field is private, so the only way to get a `Username` is through
/// [`Username::parse`]. Any function that takes a `Username` can therefore
/// rely on it being valid without checking again ("parse, don't validate").
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Username(String);

impl Username {
    pub const MAX_LENGTH: usize = 64;

    /// Trims and lowercases `input`, so `Alice` and `alice ` are one account.
    /// Allows letters, digits and `. _ - @` (so email addresses work).
    pub fn parse(input: &str) -> Result<Self, InvalidUsername> {
        let name = input.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err(InvalidUsername::Empty);
        }
        if name.len() > Self::MAX_LENGTH {
            return Err(InvalidUsername::TooLong);
        }
        match name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@')))
        {
            Some(character) => Err(InvalidUsername::Character(character)),
            None => Ok(Self(name)),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidUsername {
    #[error("a username can't be empty")]
    Empty,
    #[error("a username can be at most {} characters", Username::MAX_LENGTH)]
    TooLong,
    #[error("a username can't contain `{0}`")]
    Character(char),
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// These three impls plug `Username` into the standard conversions, so
// `"alice".parse::<Username>()` works and serde validates on deserialize.
impl FromStr for Username {
    type Err = InvalidUsername;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl TryFrom<String> for Username {
    type Error = InvalidUsername;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        Self::parse(&input)
    }
}

impl From<Username> for String {
    fn from(username: Username) -> Self {
        username.0
    }
}

/// A Lemnos account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub username: Username,
    /// What to call the user in the UI. Falls back to the username.
    pub name: String,
    pub email: Option<String>,
}

/// What is needed to create a [`User`]; the store picks the ID.
#[derive(Clone, Debug)]
pub struct NewUser {
    pub username: Username,
    pub name: Option<String>,
    pub email: Option<String>,
}

impl NewUser {
    pub fn new(username: Username) -> Self {
        Self {
            username,
            name: None,
            email: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_normalised() {
        assert_eq!(Username::parse("  Alice ").unwrap().as_str(), "alice");
        assert_eq!(
            Username::parse("a.b@example.com").unwrap().as_str(),
            "a.b@example.com"
        );
    }

    #[test]
    fn bad_usernames_are_rejected() {
        assert_eq!(Username::parse("  "), Err(InvalidUsername::Empty));
        assert_eq!(Username::parse("a b"), Err(InvalidUsername::Character(' ')));
        assert_eq!(
            Username::parse(&"a".repeat(65)),
            Err(InvalidUsername::TooLong)
        );
        assert!(serde_json::from_str::<Username>("\"a/b\"").is_err());
    }
}
