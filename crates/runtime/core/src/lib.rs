use std::path::Path;

use domain::Artifact;
use tokio::process::Command;

pub trait RuntimeAdapter: Send + Sync {
    fn build_command(&self, run_dir: &Path, artifact: &Artifact) -> Command;
}
