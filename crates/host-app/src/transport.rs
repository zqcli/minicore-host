use crate::cli::Options;
use minicore_host_core::NetworkSettings;
use minicore_host_transport::tls::{
    self,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
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
    let tls = tokio::task::spawn_blocking(move || {
        let certs = CertificateDer::pem_file_iter(cert)
            .map_err(|_| "cannot read TLS certificate chain")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "invalid TLS certificate chain")?;
        let key = PrivateKeyDer::from_pem_file(key).map_err(|_| "cannot read TLS private key")?;
        tls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map(Arc::new)
            .map_err(|_| "TLS certificate and key are invalid or mismatched")
    })
    .await
    .map_err(|_| "TLS preparation worker failed")??;
    let listener = TcpListener::bind(address)
        .await
        .map_err(|_| "cannot bind configured WSS listener")?;
    Ok(Some((listener, tls)))
}
