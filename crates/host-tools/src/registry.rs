use crate::{AgentTool, BoundLocalTools, ScopeManager, TurnPort, agent_tool_spec};
use minicore_host_local_tools::local_tool_specs;
use minicore_host_protocol::{Fault, FaultKind, ToolBinding};
use minicore_host_runtime::tools::{Tool, ToolSet, ToolSpec};
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
    Factory {
        factory: Arc<dyn ToolFactory>,
        spec: ToolSpec,
    },
    BuiltinAgent(AgentToolFactory),
    Local(ToolSpec),
}
impl RegisteredTool {
    fn spec(&self) -> &ToolSpec {
        match self {
            Self::Factory { spec, .. } => spec,
            Self::BuiltinAgent(factory) => factory.spec(),
            Self::Local(spec) => spec,
        }
    }
}
impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    /// Register an external factory. A built-in name does not change its source.
    pub fn register(&mut self, factory: Arc<dyn ToolFactory>) -> Result<(), Fault> {
        let spec = factory.spec().clone();
        self.insert(RegisteredTool::Factory { factory, spec })
    }
    /// Register the actual compiled agent implementation, retaining that fact
    /// separately from arbitrary factories with the same public name.
    pub fn register_agent_tool(&mut self, scopes: Arc<ScopeManager>) -> Result<(), Fault> {
        self.insert(RegisteredTool::BuiltinAgent(AgentToolFactory::new(scopes)))
    }
    fn insert(&mut self, entry: RegisteredTool) -> Result<(), Fault> {
        let name = entry.spec().name().as_str().to_owned();
        if self.entries.contains_key(&name) {
            return Err(Fault::new(
                FaultKind::SessionConfigInvalid,
                format!("tool {name} is already registered"),
            ));
        }
        self.entries.insert(name, entry);
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
    /// Source facts used by the production resolver's two built-in exemptions.
    /// These are not inferred from a factory's name or declaration contents.
    pub fn has_builtin_read(&self) -> bool {
        matches!(self.entries.get("read"), Some(RegisteredTool::Local(_)))
    }
    pub fn has_builtin_agent(&self) -> bool {
        matches!(
            self.entries.get("agent"),
            Some(RegisteredTool::BuiltinAgent(_))
        )
    }
    pub fn requires_local(&self, selected: &[ToolBinding]) -> bool {
        selected.iter().any(|selection| {
            matches!(
                self.entries.get(&selection.name),
                Some(RegisteredTool::Local(_))
            )
        })
    }
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.entries
            .values()
            .map(|entry| entry.spec().clone())
            .collect()
    }
    /// The exact registered declaration, without binding or executing a tool.
    pub fn spec(&self, name: &str) -> Option<&ToolSpec> {
        self.entries.get(name).map(RegisteredTool::spec)
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
                RegisteredTool::Factory { factory, .. } => factory.bind(Arc::clone(&port))?,
                RegisteredTool::BuiltinAgent(factory) => factory.bind(Arc::clone(&port))?,
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
            builder
                .register_arc_checked(tool, entry.spec())
                .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        }
        builder
            .build()
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ExternalFixture(ToolSpec);
    impl ToolFactory for ExternalFixture {
        fn spec(&self) -> &ToolSpec {
            &self.0
        }
        fn bind(&self, _: Arc<dyn TurnPort>) -> Result<Arc<dyn Tool>, Fault> {
            unreachable!("source inspection does not bind or execute tools")
        }
    }
    fn external(name: &str) -> Arc<dyn ToolFactory> {
        Arc::new(ExternalFixture(
            ToolSpec::new(
                name.parse().unwrap(),
                "external fixture",
                json!({"type":"object"}),
            )
            .unwrap(),
        ))
    }

    #[test]
    fn registration_freezes_external_factory_declarations_once() {
        struct ChangingFactory {
            specs: [ToolSpec; 2],
            reads: AtomicUsize,
        }
        impl ToolFactory for ChangingFactory {
            fn spec(&self) -> &ToolSpec {
                &self.specs[self.reads.fetch_add(1, Ordering::SeqCst).min(1)]
            }
            fn bind(&self, _: Arc<dyn TurnPort>) -> Result<Arc<dyn Tool>, Fault> {
                unreachable!("declaration lookup does not bind tools")
            }
        }
        let factory = Arc::new(ChangingFactory {
            specs: ["registered", "changed"].map(|description| {
                ToolSpec::new("external".parse().unwrap(), description, json!({})).unwrap()
            }),
            reads: AtomicUsize::new(0),
        });
        let mut registry = ToolRegistry::new();
        registry.register(factory.clone()).unwrap();
        assert_eq!(
            registry.spec("external").unwrap().description().as_str(),
            "registered"
        );
        assert_eq!(registry.specs(), vec![factory.specs[0].clone()]);
        assert_eq!(factory.reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn builtin_source_facts_come_only_from_the_actual_registration_paths() {
        let mut external_registry = ToolRegistry::new();
        external_registry.register(external("read")).unwrap();
        external_registry.register(external("agent")).unwrap();
        assert!(!external_registry.has_builtin_read());
        assert!(!external_registry.has_builtin_agent());
        assert!(external_registry.register_local_tools().is_err());
        assert!(
            external_registry
                .register_agent_tool(ScopeManager::new())
                .is_err()
        );
        assert!(!external_registry.has_builtin_read());
        assert!(!external_registry.has_builtin_agent());
        assert_eq!(external_registry.specs().len(), 2);

        let mut builtins = ToolRegistry::new();
        builtins.register_local_tools().unwrap();
        builtins.register_agent_tool(ScopeManager::new()).unwrap();
        assert!(builtins.has_builtin_read());
        assert!(builtins.has_builtin_agent());
        assert!(builtins.register(external("read")).is_err());
        assert!(builtins.register(external("agent")).is_err());
        assert!(builtins.has_builtin_read());
        assert!(builtins.has_builtin_agent());
    }
}
