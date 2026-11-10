//! Shared RPC adapters: framing, wire values, server dispatch, and client correlation.
mod client;
mod framing;
mod server;
mod wire;

pub(crate) use client::{RpcClient, RpcSubscription};
pub(crate) use framing::{FrameReader, FrameWriter};
pub use server::serve_stdio;
pub(crate) use server::{Initialization, serve_frames};
