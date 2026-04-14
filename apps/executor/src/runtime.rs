use std::path::Path;

use tokio::process::Command;

pub trait RuntimeAdapter: Send + Sync {
    fn build_command(&self, run_dir: &Path) -> Command;
}

pub struct PythonRuntimeAdapter {
    binary: String,
    entrypoint: String,
}

impl PythonRuntimeAdapter {
    pub fn new(binary: String, entrypoint: String) -> Self {
        Self { binary, entrypoint }
    }
}

impl RuntimeAdapter for PythonRuntimeAdapter {
    fn build_command(&self, run_dir: &Path) -> Command {
        let mut command = Command::new(&self.binary);
        command.arg(run_dir.join(&self.entrypoint));
        command
    }
}

pub struct NodeRuntimeAdapter {
    binary: String,
    entrypoint: String,
}

impl NodeRuntimeAdapter {
    pub fn new(binary: String, entrypoint: String) -> Self {
        Self { binary, entrypoint }
    }
}

impl RuntimeAdapter for NodeRuntimeAdapter {
    fn build_command(&self, run_dir: &Path) -> Command {
        let mut command = Command::new(&self.binary);
        command.arg(run_dir.join(&self.entrypoint));
        command
    }
}

pub fn runtime_adapter_for(
    runtime_pack_id: &str,
    default_command: String,
    default_entrypoint: String,
) -> Box<dyn RuntimeAdapter> {
    if runtime_pack_id.starts_with("node") {
        let binary = std::env::var("NODE_EXECUTOR_COMMAND").unwrap_or_else(|_| "node".to_string());
        let entrypoint =
            std::env::var("NODE_EXECUTOR_ENTRYPOINT").unwrap_or_else(|_| "index.js".to_string());
        Box::new(NodeRuntimeAdapter::new(binary, entrypoint))
    } else {
        Box::new(PythonRuntimeAdapter::new(
            default_command,
            default_entrypoint,
        ))
    }
}
