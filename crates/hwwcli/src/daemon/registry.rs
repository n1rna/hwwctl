//! Single-task registry actor.
//!
//! All mutation of the instance map happens inside one tokio task that
//! receives commands over an mpsc channel. The accept loop sends each
//! incoming control request as a [`Command`] with a oneshot reply, and
//! awaits the reply before writing the IPC response. This serializes
//! every state change so the registry is free of locks.

use std::collections::HashMap;

use control::{
    Error as CtlError, ErrorCode, InstanceId, InstanceState, InstanceSummary, StartRequest,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use super::instance::Instance;
use super::spawn;

/// Commands the accept loop sends to the registry task.
enum Command {
    Start {
        req: StartRequest,
        reply: oneshot::Sender<Result<InstanceSummary, CtlError>>,
    },
    Stop {
        instance: InstanceId,
        reply: oneshot::Sender<Result<(), CtlError>>,
    },
    Status {
        instance: Option<InstanceId>,
        reply: oneshot::Sender<Result<Vec<InstanceSummary>, CtlError>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Public handle. Cheap to clone because it's a thin wrapper around the
/// channel sender.
pub(crate) struct Registry {
    tx: mpsc::Sender<Command>,
}

impl Registry {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel::<Command>(64);
        tokio::spawn(run_loop(rx));
        Self { tx }
    }

    pub async fn start(&self, req: StartRequest) -> Result<InstanceSummary, CtlError> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Start { req, reply }).await?;
        rx.await
            .map_err(|_| internal("registry dropped reply"))
            .and_then(|r| r)
    }

    pub async fn stop(&self, instance: InstanceId) -> Result<(), CtlError> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Stop { instance, reply }).await?;
        rx.await
            .map_err(|_| internal("registry dropped reply"))
            .and_then(|r| r)
    }

    pub async fn status(
        &self,
        instance: Option<InstanceId>,
    ) -> Result<Vec<InstanceSummary>, CtlError> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Status { instance, reply }).await?;
        rx.await
            .map_err(|_| internal("registry dropped reply"))
            .and_then(|r| r)
    }

    pub async fn shutdown(&self) {
        let (reply, rx) = oneshot::channel();
        if self.send(Command::Shutdown { reply }).await.is_ok() {
            let _ = rx.await;
        }
    }

    async fn send(&self, cmd: Command) -> Result<(), CtlError> {
        self.tx
            .send(cmd)
            .await
            .map_err(|_| internal("registry task is gone"))
    }
}

fn internal(msg: &str) -> CtlError {
    CtlError::new(ErrorCode::Internal, msg)
}

async fn run_loop(mut rx: mpsc::Receiver<Command>) {
    let mut instances: HashMap<InstanceId, Instance> = HashMap::new();

    while let Some(cmd) = rx.recv().await {
        match cmd {
            Command::Start { req, reply } => {
                let result = spawn::start(&mut instances, req).await;
                let _ = reply.send(result);
            }
            Command::Stop { instance, reply } => {
                let result = stop_instance(&mut instances, &instance).await;
                let _ = reply.send(result);
            }
            Command::Status { instance, reply } => {
                let result = match instance {
                    Some(id) => match instances.get(&id) {
                        Some(inst) => Ok(vec![inst.summary()]),
                        None => Err(CtlError::new(
                            ErrorCode::InstanceNotFound,
                            format!("no instance with id '{id}'"),
                        )),
                    },
                    None => Ok(instances.values().map(Instance::summary).collect()),
                };
                let _ = reply.send(result);
            }
            Command::Shutdown { reply } => {
                let ids: Vec<_> = instances.keys().cloned().collect();
                for id in ids {
                    if let Err(e) = stop_instance(&mut instances, &id).await {
                        warn!(error = %format!("{e:?}"), %id, "stop during shutdown failed");
                    }
                }
                let _ = reply.send(());
                // Stay in the loop so a second Shutdown is a no-op
                // rather than hanging the dispatcher.
            }
        }
    }
}

async fn stop_instance(
    instances: &mut HashMap<InstanceId, Instance>,
    id: &InstanceId,
) -> Result<(), CtlError> {
    // Idempotent: unknown ids are not an error so test teardown is
    // safe to call without first checking.
    let Some(mut inst) = instances.remove(id) else {
        return Ok(());
    };
    inst.state = InstanceState::Stopping;
    info!(%id, wallet = %inst.wallet, "stopping instance");

    if let Some(handle) = inst.log_drain.take() {
        handle.abort();
    }
    if let Some(mut bridge) = inst.bridge.take() {
        if let Err(e) = bridge.stop().await {
            warn!(error = %format!("{e:#}"), %id, "bridge.stop failed");
        }
    }
    if let Err(e) = inst.emulator.stop().await {
        warn!(error = %format!("{e:#}"), %id, "emulator.stop failed");
    }
    Ok(())
}
