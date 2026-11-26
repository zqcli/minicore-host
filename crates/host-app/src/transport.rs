use crate::cli::Options;
use minicore_host_core::NetworkSettings;
use minicore_host_transport::{load_tls_server_config, tls};
use std::sync::Arc;
use tokio::net::TcpListener;

pub type Network = (TcpListener, Arc<tls::ServerConfig>);

pub async fn prepare(
    options: &Options,
    network: &NetworkSettings,
) -> Result<Option<Network>, &'static str> {
    let Some(address) = network.listen else {
        if options.tls.is_some() {
            return Err("TLS files require a configured network listener");
        }
        if !options.stdio {
            return Err("enable --stdio or configure a network listener");
        }
        return Ok(None);
    };
    let (cert, key) = options
        .tls
        .clone()
        .ok_or("configured WSS listener requires --tls-cert and --tls-key")?;
    let tls = tokio::task::spawn_blocking(move || load_tls_server_config(&cert, &key))
        .await
        .map_err(|_| "TLS preparation worker failed")??;
    let listener = TcpListener::bind(address)
        .await
        .map_err(|_| "cannot bind configured WSS listener")?;
    Ok(Some((listener, tls)))
}
