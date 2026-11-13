use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use thiserror::Error;

use super::tool::{Tool, ToolSpec};
use super::types::ToolName;

#[derive(Clone, Default)]
pub struct ToolSet {
    specs: Arc<[ToolSpec]>,
    entries: Arc<BTreeMap<ToolName, EnabledTool>>,
}

#[derive(Clone)]
pub(crate) struct EnabledTool {
    pub(crate) implementation: Arc<dyn Tool>,
    pub(crate) spec: ToolSpec,
}

#[derive(Default)]
pub struct ToolSetBuilder {
    tools: BTreeMap<ToolName, RegisteredTool>,
    error: Option<ToolSetError>,
}

struct RegisteredTool {
    tool: Arc<dyn Tool>,
    spec: ToolSpec,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ToolSetError {
    #[error("tool name is already registered")]
    DuplicateTool,
    #[error("tool specification panicked")]
    Panicked,
    #[error("bound tool declaration differs from its registered declaration")]
    SpecMismatch,
}

impl ToolSet {
    pub fn builder() -> ToolSetBuilder {
        ToolSetBuilder::default()
    }

    pub fn get(&self, name: &ToolName) -> Option<Arc<dyn Tool>> {
        self.entries
            .get(name)
            .map(|tool| Arc::clone(&tool.implementation))
    }

    pub fn specs_for(&self, enabled: &BTreeSet<ToolName>) -> Vec<ToolSpec> {
        enabled
            .iter()
            .filter_map(|name| self.entries.get(name).map(|tool| tool.spec.clone()))
            .collect()
    }

    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    /// Wraps the selected implementations without reading their declarations
    /// again. The original names, ordering and frozen specifications remain.
    pub fn map_tools(
        &self,
        mut map: impl FnMut(Arc<dyn Tool>, &ToolSpec) -> Arc<dyn Tool>,
    ) -> Self {
        let entries = self
            .entries
            .iter()
            .map(|(name, tool)| {
                (
                    name.clone(),
                    EnabledTool {
                        implementation: map(Arc::clone(&tool.implementation), &tool.spec),
                        spec: tool.spec.clone(),
                    },
                )
            })
            .collect();
        Self {
            specs: Arc::clone(&self.specs),
            entries: Arc::new(entries),
        }
    }

    pub(crate) fn shared_specs(&self) -> Arc<[ToolSpec]> {
        Arc::clone(&self.specs)
    }

    pub(crate) fn entry(&self, name: &ToolName) -> Option<&EnabledTool> {
        self.entries.get(name)
    }
}

impl ToolSetBuilder {
    pub fn register<T>(&mut self, tool: T) -> &mut Self
    where
        T: Tool + 'static,
    {
        self.register_arc(Arc::new(tool))
    }

    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) -> &mut Self {
        self.register_tool(tool, None)
    }

    /// Checks and freezes the same declaration returned by the bound tool.
    pub fn register_arc_checked(
        &mut self,
        tool: Arc<dyn Tool>,
        expected: &ToolSpec,
    ) -> Result<&mut Self, ToolSetError> {
        self.register_tool(tool, Some(expected));
        match self.error {
            Some(error) => Err(error),
            None => Ok(self),
        }
    }

    fn register_tool(&mut self, tool: Arc<dyn Tool>, expected: Option<&ToolSpec>) -> &mut Self {
        if self.error.is_some() {
            return self;
        }
        let spec = match catch_unwind(AssertUnwindSafe(|| tool.spec().clone())) {
            Ok(spec) => spec,
            Err(_) => {
                self.error = Some(ToolSetError::Panicked);
                return self;
            }
        };
        if expected.is_some_and(|expected| expected != &spec) {
            self.error = Some(ToolSetError::SpecMismatch);
            return self;
        }
        let name = spec.name().clone();
        if self.tools.contains_key(&name) {
            self.error = Some(ToolSetError::DuplicateTool);
            return self;
        }
        self.tools.insert(name, RegisteredTool { tool, spec });
        self
    }

    pub fn build(self) -> Result<ToolSet, ToolSetError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let mut specs = Vec::with_capacity(self.tools.len());
        let mut entries = BTreeMap::new();
        for (name, registered) in self.tools {
            specs.push(registered.spec.clone());
            entries.insert(
                name,
                EnabledTool {
                    implementation: registered.tool,
                    spec: registered.spec,
                },
            );
        }
        Ok(ToolSet {
            specs: specs.into(),
            entries: Arc::new(entries),
        })
    }
}
