use std::{path::PathBuf, process::Stdio};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    task,
};

async fn run_command(command: &str, args: &[&str]) -> anyhow::Result<()> {
    let mut child = Command::new(command)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("failed to capture stdout");
    let stderr = child.stderr.take().expect("failed to capture stderr");

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    let stdout_task = task::spawn(async move {
        while let Some(line) = stdout_reader.next_line().await.unwrap_or(None) {
            log::info!("{}", line);
        }
    });

    let stderr_task = task::spawn(async move {
        while let Some(line) = stderr_reader.next_line().await.unwrap_or(None) {
            // docker logs to stderr ... -.-
            log::info!("{}", line);
        }
    });

    let status = child.wait().await?;

    stdout_task.await?;
    stderr_task.await?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("command failed with status: {:?}", status))
    }
}

pub(crate) async fn pull_image(image: &str) -> anyhow::Result<()> {
    run_command(
        "sh",
        &[
            "-c",
            &format!("docker images -q '{image}' | grep -q . || docker pull '{image}'"),
        ],
    )
    .await
}

pub(crate) async fn build_image(name: &str, path: &str) -> anyhow::Result<()> {
    let dockerfile = PathBuf::from(path);
    if !dockerfile.exists() {
        return Err(anyhow::anyhow!("dockerfile '{}' does not exist", path));
    } else if !dockerfile.is_file() {
        return Err(anyhow::anyhow!("path '{}' is not a dockerfile", path));
    }

    log::info!("building image '{}' from '{}'", name, dockerfile.display());

    run_command(
        "sh",
        &[
            "-c",
            &format!(
                // TODO: check if using '.' is correct in this case
                "docker build -f '{}' -t '{name}' --quiet .",
                dockerfile.display()
            ),
        ],
    )
    .await
}