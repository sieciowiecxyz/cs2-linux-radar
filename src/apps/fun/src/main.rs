#![forbid(unsafe_code)]

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tracing::info;

fn main() -> Result<()> {
    let _ = shared_logging::init("info");
    parse_args_from(env::args().skip(1))?;
    let workspace_root = workspace_root();
    let target_dir = target_dir(&workspace_root);
    let radar_bin = binary_path(&target_dir, "fun-radar");
    let trigger_bin = binary_path(&target_dir, "fun-trigger");
    let mouse_bin = binary_path(&target_dir, "fun-mouse");

    ensure_binaries(&workspace_root, &radar_bin, &trigger_bin, &mouse_bin)?;

    let children = vec![
        ChildProcess::new("fun-radar", spawn_radar(&radar_bin)?),
        ChildProcess::new("fun-trigger", spawn_trigger(&trigger_bin)?),
        ChildProcess::new("fun-mouse", spawn_mouse(&mouse_bin)?),
    ];

    info!(
        radar_bin = %radar_bin.display(),
        trigger_bin = %trigger_bin.display(),
        mouse_bin = %mouse_bin.display(),
        "fun launched child processes"
    );

    supervise_children(children)
}

fn parse_args_from<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "cs2" => {}
            "--help" | "-h" => {
                print!("{}", usage());
                std::process::exit(0);
            }
            other => bail!("unsupported argument `{other}`\n\n{}", usage()),
        }
    }
    Ok(())
}

fn usage() -> &'static str {
    "usage:\n  fun [cs2]\n\nnotes:\n  launches fun-radar, fun-trigger and fun-mouse as separate child processes.\n"
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

fn target_dir(workspace_root: &Path) -> PathBuf {
    if let Some(value) = env::var_os("CARGO_TARGET_DIR") {
        return PathBuf::from(value);
    }
    workspace_root.join("target")
}

fn binary_path(target_dir: &Path, name: &str) -> PathBuf {
    target_dir
        .join("debug")
        .join(format!("{name}{}", env::consts::EXE_SUFFIX))
}

fn ensure_binaries(
    workspace_root: &Path,
    radar_bin: &Path,
    trigger_bin: &Path,
    mouse_bin: &Path,
) -> Result<()> {
    info!(
        radar_exists = radar_bin.exists(),
        trigger_exists = trigger_bin.exists(),
        mouse_exists = mouse_bin.exists(),
        "fun is building child binaries"
    );
    let status = Command::new("cargo")
        .current_dir(workspace_root)
        .args([
            "build",
            "-q",
            "-p",
            "fun-radar",
            "-p",
            "fun-trigger",
            "-p",
            "fun-mouse",
        ])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("spawn cargo build for fun child binaries")?;
    if !status.success() {
        bail!("cargo build for fun-radar/fun-trigger/fun-mouse failed with {status}");
    }
    Ok(())
}

fn spawn_radar(binary: &Path) -> Result<Child> {
    Command::new(binary)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn radar child {}", binary.display()))
}

fn spawn_trigger(binary: &Path) -> Result<Child> {
    Command::new(binary)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn trigger child {}", binary.display()))
}

fn spawn_mouse(binary: &Path) -> Result<Child> {
    Command::new(binary)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn mouse child {}", binary.display()))
}

struct ChildProcess {
    name: &'static str,
    child: Child,
}

impl ChildProcess {
    fn new(name: &'static str, child: Child) -> Self {
        Self { name, child }
    }
}

fn supervise_children(mut children: Vec<ChildProcess>) -> Result<()> {
    loop {
        for idx in 0..children.len() {
            if let Some(status) = children[idx]
                .child
                .try_wait()
                .with_context(|| format!("wait for {} child", children[idx].name))?
            {
                let name = children[idx].name;
                terminate_siblings(&mut children, idx);
                return exit_for_child(name, status);
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn terminate_siblings(children: &mut [ChildProcess], exited_idx: usize) {
    for (idx, child) in children.iter_mut().enumerate() {
        if idx == exited_idx {
            continue;
        }
        match child.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                let _ = child.child.kill();
                let _ = child.child.wait();
                info!(child = child.name, "terminated sibling child process");
            }
            Err(err) => {
                info!(
                    child = child.name,
                    error = %err,
                    "failed to inspect sibling child process"
                );
            }
        }
    }
}

fn exit_for_child(name: &str, status: ExitStatus) -> Result<()> {
    if status.success() {
        info!(child = name, %status, "child process exited cleanly");
        return Ok(());
    }
    bail!("{name} exited with {status}");
}

#[cfg(test)]
mod tests {
    use super::{parse_args_from, usage};

    #[test]
    fn usage_no_longer_mentions_mouse_transform() {
        assert!(!usage().contains("--mouse-transform"));
    }

    #[test]
    fn parse_args_accepts_empty() {
        assert!(parse_args_from(Vec::<String>::new()).is_ok());
    }
}
