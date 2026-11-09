use crate::{
    cli::Options,
    diagnostics::{self, Diagnostics},
    signals::Signals,
    transport,
};
use minicore_host_core::{Host, HostConfig, HostProcessControl, HostProcessSnapshot};
use minicore_host_diagnostics::{Context, EventData};
use minicore_host_protocol::{
    Fault, FaultKind, HostLifecycle, HostShutdownRequest, HostShutdownStrategy, OperationId,
};
use minicore_host_resources::HostResourceBuilder;
use std::sync::Arc;
use tokio::task::JoinSet;

pub async fn run(
    options: Options,
    initial: HostConfig,
    diagnostics: &mut Option<Diagnostics>,
) -> Result<(), &'static str> {
    let mut signals = Signals::new().map_err(|_| "cannot initialize process signals")?;
    let mut network = None;
    let mut preparation_error = None;
    let host = Host::open_configured(&options.data_root, initial, |config| async {
        let prepare = async {
            *diagnostics = Some(
                tokio::task::spawn_blocking({
                    let settings = config.logging.clone();
                    move || Diagnostics::install(settings)
                })
                .await
                .map_err(|_| "diagnostic initialization worker failed")??,
            );
            network = transport::prepare(&options, &config.network).await?;
            Ok::<_, &'static str>(())
        }
        .await;
        if let Err(message) = prepare {
            preparation_error = Some(message);
            return Err(Fault::new(FaultKind::HostConfigInvalid, message));
        }
        HostResourceBuilder::builtins()?.prepare(config).await
    })
    .await
    .map_err(|fault| {
        diagnostics::fault(diagnostics::STARTUP_FAILED, Context::default(), &fault);
        preparation_error.unwrap_or(
            "Host initialization failed; check config, credentials and exclusive data-root access",
        )
    })?;
    let control = host.process_control();
    let mut current = control
        .snapshot()
        .map_err(|_| "Host lifecycle is unavailable")?;
    let context = context(&current);
    let listen_port = network
        .as_ref()
        .and_then(|(listener, _)| listener.local_addr().ok())
        .map(|address| u64::from(address.port()));
    let wss = network.is_some();
    let mut transports = JoinSet::new();
    if options.stdio {
        let host = Arc::clone(&host);
        transports.spawn(async move {
            (
                "stdio",
                minicore_host_transport::serve_stdio(host, tokio::io::stdin(), tokio::io::stdout())
                    .await
                    .is_ok(),
            )
        });
    }
    if let Some((listener, tls)) = network {
        let host = Arc::clone(&host);
        transports.spawn(async move {
            (
                "wss",
                minicore_host_transport::serve_wss(host, listener, tls)
                    .await
                    .is_ok(),
            )
        });
    }
    diagnostics::STARTED.emit(|| {
        let mut data = EventData::default()
            .context(context.clone())
            .attribute("stdio", options.stdio)
            .attribute("wss", wss);
        if let Some(port) = listen_port {
            data = data.attribute("listen_port", port);
        }
        data
    });
    let mut failed = false;
    while current.lifecycle != HostLifecycle::Stopped {
        tokio::select! {
            next = control.changed(&current) => {
                current = next.map_err(|_| "Host lifecycle supervision failed")?;
                diagnostics::STATE.emit(|| EventData::default().context(context.clone())
                    .attribute("lifecycle", lifecycle(current.lifecycle))
                    .attribute("strategy", strategy(current.shutdown_strategy)));
                if let Some(fault) = &current.shutdown_fault {
                    diagnostics::fault(diagnostics::SHUTDOWN_BLOCKED, context.clone(), fault);
                }
            }
            _ = signals.recv() => {
                request_shutdown(&control, &context, "signal")?;
            }
            Some(result) = transports.join_next(), if !transports.is_empty() => {
                match result {
                    Ok(("stdio", success)) => {
                        if !success {
                            failed = true;
                            diagnostics::TRANSPORT_FAILED.emit(|| EventData::default().context(context.clone()).attribute("transport", "stdio"));
                        }
                        diagnostics::DISCONNECTED.emit(|| EventData::default().context(context.clone()));
                    },
                    Ok((name, _)) => {
                        failed = true;
                        diagnostics::TRANSPORT_FAILED.emit(|| EventData::default().context(context.clone()).attribute("transport", name));
                        request_shutdown(&control, &context, "transport_failure")?;
                    }
                    Err(_) => {
                        failed = true;
                        diagnostics::TRANSPORT_FAILED.emit(|| EventData::default().context(context.clone()).attribute("transport", "task"));
                        request_shutdown(&control, &context, "transport_failure")?;
                    }
                }
            }
        }
    }
    // Transport cancellation cannot cancel business work: the actual Host
    // stopped barrier has already retired every accepted execution and save.
    transports.abort_all();
    while transports.join_next().await.is_some() {}
    diagnostics::STOPPED.emit(|| EventData::default().context(context));
    drop(host);
    if failed {
        Err("transport failure; Host completed controlled cleanup")
    } else {
        Ok(())
    }
}

fn request_shutdown(
    control: &HostProcessControl,
    context: &Context,
    reason: &'static str,
) -> Result<(), &'static str> {
    let operation_id =
        OperationId::new().map_err(|_| "cannot allocate shutdown operation identity")?;
    diagnostics::SHUTDOWN_REQUESTED.emit(|| {
        EventData::default()
            .context(Context {
                operation_id: Some(operation_id.clone()),
                ..context.clone()
            })
            .attribute("reason", reason)
    });
    control
        .shutdown(HostShutdownRequest {
            operation_id,
            strategy: HostShutdownStrategy::Graceful,
        })
        .map_err(|_| "Host did not accept controlled shutdown")?;
    Ok(())
}
fn context(snapshot: &HostProcessSnapshot) -> Context {
    Context {
        host_id: Some(snapshot.host_id.clone()),
        host_incarnation: Some(snapshot.host_incarnation.clone()),
        ..Context::default()
    }
}
fn lifecycle(value: HostLifecycle) -> &'static str {
    match value {
        HostLifecycle::Starting => "starting",
        HostLifecycle::Running => "running",
        HostLifecycle::Draining => "draining",
        HostLifecycle::Stopping => "stopping",
        HostLifecycle::Stopped => "stopped",
    }
}
fn strategy(value: Option<HostShutdownStrategy>) -> &'static str {
    match value {
        None => "none",
        Some(HostShutdownStrategy::Graceful) => "graceful",
        Some(HostShutdownStrategy::Force) => "force",
    }
}
