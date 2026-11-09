//! MiniCore Host v4.93 shared value contracts.
//!
//! These types do not authorize callers, schedule work, or promise availability.
//! Direct owners validate authorization and resource references at acceptance.
//! Accepted receipts describe facts at formation, never execution completion.

#![forbid(unsafe_code)]

mod config;
mod fault;
mod id;
mod ownership;

pub use config::*;
pub use fault::*;
pub use id::*;
pub use ownership::*;

/// Unsigned, monotonically increasing versions, scoped to the owning object and
/// Host incarnation. Owners must use checked arithmetic rather than wraparound.
pub type Revision = u64;
pub type ConfigRevision = Revision;
pub type ControllerRevision = Revision;

/// A structural validation error. It is not itself a business Fault; the owner
/// maps it according to the operation whose candidate it was validating.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for ValidationError {}

pub(crate) fn invalid(message: impl Into<String>) -> ValidationError {
    ValidationError(message.into())
}

pub(crate) fn present<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

// Cross-field validated DTOs reject invalid combinations in both directions.
macro_rules! validated_dto {
    ($(#[$meta:meta])* $name:ident, $wire:ident, $wire_name:literal { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name { $(pub $field: $ty),* }
        #[derive(serde::Serialize, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire { $($(#[$attr])* $field: $ty),* }
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok,S::Error> {
                self.validate().map_err(serde::ser::Error::custom)?;
                $wire { $($field: self.$field.clone()),* }.serialize(serializer)
            }
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self,D::Error> {
                let wire = $wire::deserialize(deserializer)?;
                let value = Self { $($field: wire.$field),* };
                value.validate().map_err(serde::de::Error::custom)?;
                Ok(value)
            }
        }
    };
}
pub(crate) use validated_dto;

mod message;
pub use message::*;

mod receipt;
pub use receipt::*;

mod control;
pub use control::*;

mod catalog;
pub use catalog::*;

mod rpc;
pub use rpc::*;

// Option-valued fields that must be present (with null when no value exists).
pub(crate) fn nullable<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    <Option<T> as serde::Deserialize>::deserialize(deserializer)
}

mod observation;
pub use observation::*;
