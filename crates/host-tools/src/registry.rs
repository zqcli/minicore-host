use crate::{AgentTool, BoundLocalTools, ScopeManager, TurnPort, agent_tool_spec};
use minicore_host_local_tools::local_tool_specs;
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
    entries: BTreeMap<String, RegisteredTool>,
}
enum RegisteredTool {
    Factory(Arc<dyn ToolFactory>),
    Local(ToolSpec),
}
impl RegisteredTool {
    fn spec(&self) -> &ToolSpec {
        match self {
            Self::Factory(factory) => factory.spec(),
            Self::Local(spec) => spec,
        }
    }
}
impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, factory: Arc<dyn ToolFactory>) -> Result<(), Fault> {
        let name = factory.spec().name().as_str().to_owned();
        if self.entries.contains_key(&name) {
            return Err(Fault::new(
                FaultKind::SessionConfigInvalid,
                format!("tool {name} is already registered"),
            ));
        }
        self.entries.insert(name, RegisteredTool::Factory(factory));
        Ok(())
    }
    /// Register the compiled declarations, without enabling them in a Session.
    /// Actual binding still requires its frozen Workspace and real record sink.
    pub fn register_local_tools(&mut self) -> Result<(), Fault> {
        let specs = local_tool_specs();
        for spec in &specs {
            if self.entries.contains_key(spec.name().as_str()) {
                return Err(Fault::new(
                    FaultKind::SessionConfigInvalid,
                    format!("tool {} is already registered", spec.name()),
                ));
            }
        }
        for spec in specs {
            self.entries
                .insert(spec.name().as_str().to_owned(), RegisteredTool::Local(spec));
        }
        Ok(())
    }
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.entries
            .values()
            .map(|entry| entry.spec().clone())
            .collect()
    }
    pub fn bind(
        &self,
        selected: &[ToolBinding],
        port: Arc<dyn TurnPort>,
    ) -> Result<ToolSet, Fault> {
        self.bind_selected(selected, port, None)
    }
    pub fn bind_with_local(
        &self,
        selected: &[ToolBinding],
        port: Arc<dyn TurnPort>,
        local: &BoundLocalTools,
    ) -> Result<ToolSet, Fault> {
        self.bind_selected(selected, port, Some(local))
    }
    fn bind_selected(
        &self,
        selected: &[ToolBinding],
        port: Arc<dyn TurnPort>,
        local: Option<&BoundLocalTools>,
    ) -> Result<ToolSet, Fault> {
        let mut builder = ToolSet::builder();
        for selection in selected {
            let entry = self.entries.get(&selection.name).ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionResourceMissing,
                    format!("tool {} is not registered", selection.name),
                )
            })?;
            let tool = match entry {
                RegisteredTool::Factory(factory) => factory.bind(Arc::clone(&port))?,
                RegisteredTool::Local(_) => local
                    .and_then(|local| local.tool(&selection.name))
                    .ok_or_else(|| {
                        Fault::new(
                            FaultKind::SessionResourceMissing,
                            format!(
                                "tool {} requires its bound workspace and actual record owner",
                                selection.name
                            ),
                        )
                    })?,
            };
            if tool.spec() != entry.spec() {
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
