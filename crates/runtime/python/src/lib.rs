use std::path::Path;

use domain::Artifact;
use runtime_core::RuntimeAdapter;
use tokio::process::Command;

pub struct PythonRuntimeAdapter {
    binary: String,
}

impl PythonRuntimeAdapter {
    pub fn new(binary: String) -> Self {
        Self { binary }
    }
}

impl RuntimeAdapter for PythonRuntimeAdapter {
    fn build_command(&self, run_dir: &Path, artifact: &Artifact) -> Command {
        let mut command = Command::new(&self.binary);
        command.arg(run_dir.join(&artifact.entrypoint));
        command
    }
}
