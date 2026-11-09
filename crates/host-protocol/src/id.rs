use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

/// The exact 32-symbol alphabet from SPEC §2. Each generated symbol carries five
/// independent random bits; there are no type, parent, time, or device prefixes.
pub const ID_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
pub const ID_LENGTH: usize = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidId;
impl fmt::Display for InvalidId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ID must contain exactly 20 lowercase characters from the MiniCore alphabet")
    }
}
impl std::error::Error for InvalidId {}

#[derive(Debug)]
pub struct IdGenerationError(getrandom::Error);
impl fmt::Display for IdGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "system secure random source failed: {}", self.0)
    }
}
impl std::error::Error for IdGenerationError {}

/// Thin local ID generation. Failure is returned; no timestamps, weak RNG, or
/// synthetic fallback is used. Masking is unbiased because 32 divides 256.
pub fn new_id() -> Result<String, IdGenerationError> {
    let mut random = [0_u8; ID_LENGTH];
    getrandom::fill(&mut random).map_err(IdGenerationError)?;
    Ok(encode_random(random))
}

fn encode_random(random: [u8; ID_LENGTH]) -> String {
    random
        .into_iter()
        .map(|byte| char::from(ID_ALPHABET[usize::from(byte & 31)]))
        .collect()
}

fn validate_id(value: &str) -> Result<(), InvalidId> {
    if value.len() == ID_LENGTH && value.bytes().all(|byte| ID_ALPHABET.contains(&byte)) {
        Ok(())
    } else {
        Err(InvalidId)
    }
}

macro_rules! ids {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);
        impl $name {
            pub fn new() -> Result<Self, IdGenerationError> { new_id().map(Self) }
            pub fn as_str(&self) -> &str { &self.0 }
        }
        impl FromStr for $name {
            type Err = InvalidId;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                validate_id(value)?;
                Ok(Self(value.to_owned()))
            }
        }
        impl TryFrom<String> for $name {
            type Error = InvalidId;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                validate_id(&value)?;
                Ok(Self(value))
            }
        }
        impl AsRef<str> for $name { fn as_ref(&self) -> &str { self.as_str() } }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(&self.0) }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d)?.try_into().map_err(serde::de::Error::custom)
            }
        }
    )+};
}
ids!(
    HostId,
    HostIncarnation,
    AgentId,
    SessionId,
    TurnId,
    LoadId,
    MessageId,
    OperationId,
    SubscriptionId,
    InteractionId
);

/// Upstream ToolCall identifiers are preserved exactly, not normalized or
/// interpreted as globally unique. Locate them together with their TurnId.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolCallId(pub String);
impl ToolCallId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl From<String> for ToolCallId {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_symbol_has_exactly_eight_byte_preimages() {
        let mut counts = [0; 32];
        for byte in 0..=255_u8 {
            let encoded = encode_random([byte; ID_LENGTH]);
            let index = ID_ALPHABET
                .iter()
                .position(|c| *c == encoded.as_bytes()[0])
                .unwrap();
            counts[index] += 1;
        }
        assert_eq!(counts, [8; 32]);
    }
}
