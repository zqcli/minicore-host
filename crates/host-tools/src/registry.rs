use crate::{AgentTool, ScopeManager, TurnPort, agent_tool_spec};
use minicore_host_protocol::{Fault, FaultKind, ToolBinding};
use minicore_runtime::tools::{Tool, ToolSet, ToolSpec};
use std::{collections::BTreeMap, sync::Arc};

/// Static deployment registration; binding supplies only the selected Turn's
/// narrow capabilities. The resulting ToolSet is frozen for the public Turn.
pub trait ToolFactory: Send + Sync + 'static {
    fn spec(&self) -> &ToolSpec;
    fn bind(&self, port: Arc<dyn TurnPort>) -> Result<Arc<dyn Tool>, Fault>;
}
pub struct AgentToolFactory {
    spec: ToolSpec,
    scopes: Arc<ScopeManager>,
}
impl AgentToolFactory {
    pub fn new(scopes: Arc<ScopeManager>) -> Self {
        Self {
            spec: agent_tool_spec(),
            scopes,
        }
    }
}
impl ToolFactory for AgentToolFactory {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn bind(&self, port: Arc<dyn TurnPort>) -> Result<Arc<dyn Tool>, Fault> {
        Ok(Arc::new(AgentTool::new(port, Arc::clone(&self.scopes))))
    }
}
#[derive(Default)]
pub struct ToolRegistry {
    factories: BTreeMap<String, Arc<dyn ToolFactory>>,
}
impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, factory: Arc<dyn ToolFactory>) -> Result<(), Fault> {
        let name = factory.spec().name().as_str().to_owned();
        if self.factories.contains_key(&name) {
            return Err(Fault::new(
                FaultKind::SessionConfigInvalid,
                format!("tool {name} is already registered"),
            ));
        }
        self.factories.insert(name, factory);
        Ok(())
    }
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.factories
            .values()
            .map(|factory| factory.spec().clone())
            .collect()
    }
    pub fn bind(
        &self,
        selected: &[ToolBinding],
        port: Arc<dyn TurnPort>,
    ) -> Result<ToolSet, Fault> {
        let mut builder = ToolSet::builder();
        for selection in selected {
            let factory = self.factories.get(&selection.name).ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionResourceMissing,
                    format!("tool {} is not registered", selection.name),
                )
            })?;
            let tool = factory.bind(Arc::clone(&port))?;
            if tool.spec() != factory.spec() {
                return Err(Fault::new(
                    FaultKind::SessionConfigInvalid,
                    "bound tool declaration differs from its registered declaration",
                ));
            }
            builder.register_arc(tool);
        }
        builder
            .build()
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))
    }
}
