#![allow(dead_code)]
//! Real production preparation and transports. Only credentials and scheduling
//! are fixtures; no candidate/result is manufactured by the adapter.
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_provider::SecretApiKey;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use minicore_host_transport::*;
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf},
    sync::Semaphore,
    task::JoinHandle,
};

pub const KEY_A: &str = "task035-fixture-only-key-a";
pub const KEY_B: &str = "task035-fixture-only-key-b";
pub const MODEL_KEY: &str = "task035-fixture-only-model-key";
pub const SOURCE_A: &str = "TASK035_A";
pub const SOURCE_ALIAS_A: &str = "TASK035_ALIAS_A";
pub const SOURCE_B: &str = "TASK035_B";

pub fn options() -> CallOptions {
    CallOptions {
        wait_timeout_ms: Some(5_000),
    }
}
pub fn request(candidate: HostConfig, expected_revision: u64) -> HostReloadRequest<HostConfig> {
    HostReloadRequest {
        operation_id: OperationId::new().unwrap(),
        expected_revision,
        candidate,
    }
}
pub fn fault(error: CallError) -> Fault {
    match error {
        CallError::Business(fault) => fault,
        other => panic!("expected a business fault, got {other:?}"),
    }
}
pub fn config() -> HostConfig {
    HostConfig {
        network: NetworkSettings {
            listen: None,
            access_key_env: Some(SOURCE_A.into()),
        },
        ..HostConfig::default()
    }
}
pub fn credentials(config: &HostConfig) -> HostCredentials {
    let key = config
        .network
        .access_key_env
        .as_deref()
        .map(|source| match source {
            SOURCE_A | SOURCE_ALIAS_A => KEY_A,
            SOURCE_B => KEY_B,
            _ => panic!("unknown fixture credential source"),
        });
    HostCredentials {
        model_keys: config
            .models
            .iter()
            .map(|model| (model.model.clone(), SecretApiKey::new(MODEL_KEY).unwrap()))
            .collect(),
        network_access_key: key.map(str::to_owned),
    }
}
/// An explicit test credential source instead of mutating process environment.
/// Both initial and reload resources use the same production static registry.
pub struct CredentialFixture {
    pub builder: HostResourceBuilder,
    pub calls: AtomicUsize,
    pub entered: Semaphore,
    pub release: Semaphore,
    gated: bool,
}
impl CredentialFixture {
    pub async fn host(config: HostConfig, gated: bool) -> (Arc<Host>, Arc<Self>) {
        let preparer = Arc::new(Self {
            builder: HostResourceBuilder::builtins().unwrap(),
            calls: AtomicUsize::new(0),
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
            gated,
        });
        let candidate = preparer
            .builder
            .prepare_with_credentials(config.clone(), credentials(&config))
            .await
            .unwrap()
            .with_preparer(preparer.clone());
        (Host::memory_configured(candidate).unwrap(), preparer)
    }
    pub async fn started(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
    pub fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl HostConfigurationPreparer for CredentialFixture {
    fn prepare(&self, config: HostConfig) -> HostConfigurationFuture<'_> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.add_permits(1);
            if self.gated {
                self.release.acquire().await.unwrap().forget();
            }
            let credentials = credentials(&config);
            self.builder
                .prepare_with_credentials(config, credentials)
                .await
        })
    }
}
pub async fn terminal(client: &HostClient, operation: OperationId) -> HostReloadReceipt {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Receipt::HostReload(receipt) =
                client.receipt(operation.clone(), options()).await.unwrap()
            else {
                panic!("wrong direct owner receipt");
            };
            if !matches!(
                receipt.result,
                HostReloadResult::Preparing { .. } | HostReloadResult::Applying { .. }
            ) {
                return receipt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
pub async fn revision(client: &HostClient, expected: u64) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client.config(options()).await.unwrap().config_revision == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
pub use super::wss_common::tls_server;
pub async fn stdio(host: Arc<Host>) -> (HostClient, JoinHandle<std::io::Result<()>>) {
    let (peer, server) = tokio::io::duplex(256);
    let (reader, writer) = tokio::io::split(peer);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(serve_stdio(host, input, output));
    (
        HostClient::stdio(reader, writer, options()).await.unwrap(),
        task,
    )
}
pub struct Connected {
    pub client: HostClient,
    pipe: Option<JoinHandle<std::io::Result<()>>>,
    _tls: Option<super::wss_common::Server>,
}
impl Connected {
    pub async fn new(host: Arc<Host>, transport: u8) -> Self {
        match transport {
            0 => Self {
                client: HostClient::in_process(host).unwrap(),
                pipe: None,
                _tls: None,
            },
            1 => {
                let (client, pipe) = stdio(host).await;
                Self {
                    client,
                    pipe: Some(pipe),
                    _tls: None,
                }
            }
            2 => {
                let tls = tls_server(host).await;
                let client = tls.client(KEY_A).await.unwrap();
                Self {
                    client,
                    pipe: None,
                    _tls: Some(tls),
                }
            }
            _ => unreachable!(),
        }
    }
    pub async fn finish(self) {
        self.client.close();
        if let Some(pipe) = self.pipe {
            pipe.await.unwrap().unwrap();
        }
    }
}

pub struct RawStdio {
    pub reader: BufReader<ReadHalf<DuplexStream>>,
    pub writer: WriteHalf<DuplexStream>,
    task: JoinHandle<std::io::Result<()>>,
}
impl RawStdio {
    pub fn new(host: Arc<Host>) -> Self {
        let (peer, server) = tokio::io::duplex(256);
        let (reader, writer) = tokio::io::split(peer);
        let (input, output) = tokio::io::split(server);
        Self {
            reader: BufReader::new(reader),
            writer,
            task: tokio::spawn(serve_stdio(host, input, output)),
        }
    }
    pub async fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        let frame =
            serde_json::json!({ "jsonrpc":"2.0", "id":id, "method":method, "params":params });
        self.writer
            .write_all(&serde_json::to_vec(&frame).unwrap())
            .await
            .unwrap();
        self.writer.write_all(b"\n").await.unwrap();
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(5), self.reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    pub async fn finish(mut self) {
        self.writer.shutdown().await.unwrap();
        drop(self.writer);
        self.task.await.unwrap().unwrap();
    }
}
