use std::path::Path;

use domain::Artifact;
use runtime_core::RuntimeAdapter;
use tokio::process::Command;

pub struct NodeRuntimeAdapter {
    binary: String,
}

impl NodeRuntimeAdapter {
    pub fn new(binary: String) -> Self {
        Self { binary }
    }
}

impl RuntimeAdapter for NodeRuntimeAdapter {
    fn build_command(&self, run_dir: &Path, artifact: &Artifact) -> Command {
        let mut command = Command::new(&self.binary);
        command.arg(run_dir.join(&artifact.entrypoint));
        command
    }
}
