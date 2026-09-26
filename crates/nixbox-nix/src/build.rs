use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub enum BuildEvent {
    Line(String),
    Finished(Result<(), String>),
    Cancelled,
}

fn find_in_nix_profiles(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER").unwrap_or_default();
    let candidates = [
        format!("/run/wrappers/bin/{name}"),
        format!("{home}/.nix-profile/bin/{name}"),
        format!("/etc/profiles/per-user/{user}/bin/{name}"),
        format!("/run/current-system/sw/bin/{name}"),
        format!("/nix/var/nix/profiles/default/bin/{name}"),
    ];
    candidates
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// Returns (command, args) for `home-manager switch --flake <config_dir>#<user>`.
/// Falls back to `nix run nixpkgs#home-manager -- switch` when the binary is absent.
pub fn home_manager_switch_cmd(config_dir: &Path) -> (String, Vec<String>) {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let flake_ref = format!("{}#{}", config_dir.display(), user);
    if let Some(bin) = find_in_nix_profiles("home-manager") {
        (
            bin.to_string_lossy().into_owned(),
            vec!["switch".into(), "--flake".into(), flake_ref],
        )
    } else {
        (
            "nix".into(),
            vec![
                "run".into(),
                "nixpkgs#home-manager".into(),
                "--".into(),
                "switch".into(),
                "--flake".into(),
                flake_ref,
            ],
        )
    }
}

/// Checks whether the flake at `config_dir` exposes a standalone
/// `homeConfigurations.<user>` output. When false, home-manager packages must
/// be applied via `nixos-rebuild` because the user wires home-manager in as a
/// NixOS module rather than as a separate flake output.
pub async fn flake_has_home_configuration(config_dir: &Path) -> bool {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let nix = find_in_nix_profiles("nix").unwrap_or_else(|| PathBuf::from("nix"));
    let expr = format!(
        "let f = builtins.getFlake \"{}\"; in f ? homeConfigurations && f.homeConfigurations ? \"{}\"",
        config_dir.display(),
        user,
    );
    let output = Command::new(nix)
        .args(["eval", "--impure", "--json", "--expr", &expr])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;
    match output {
        Ok(out) if out.status.success() => std::str::from_utf8(&out.stdout)
            .map(|s| s.trim() == "true")
            .unwrap_or(false),
        _ => false,
    }
}

pub fn nixos_rebuild_switch_cmd(config_dir: &Path) -> (String, Vec<String>) {
    let flake_ref = format!("{}#nixos", config_dir.display());
    (
        "sudo".into(),
        vec![
            "nixos-rebuild".into(),
            "switch".into(),
            "--flake".into(),
            flake_ref,
        ],
    )
}

/// Runs `command_name` with `args`, streaming its output to `tx` until it
/// exits or `cancel` fires. `envs` is added to the inherited environment;
/// a desktop front-end uses it to hand sudo an askpass helper.
pub async fn rebuild(
    command_name: &str,
    args: &[&str],
    envs: &[(String, String)],
    tx: mpsc::Sender<BuildEvent>,
    mut cancel: oneshot::Receiver<()>,
) -> Result<()> {
    let resolved =
        find_in_nix_profiles(command_name).unwrap_or_else(|| PathBuf::from(command_name));

    let mut process = Command::new(&resolved);
    process
        .args(args)
        .envs(envs.iter().map(|(key, value)| (key, value)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Put wrappers such as sudo and every descendant they spawn in one group.
    // This lets cancellation stop nixos-rebuild itself, not only its parent.
    unsafe {
        process.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = process.spawn().with_context(|| {
        format!(
            "spawning `{}` (resolved: {})",
            command_name,
            resolved.display()
        )
    })?;

    let stdout = child.stdout.take().context("capturing stdout")?;
    let stderr = child.stderr.take().context("capturing stderr")?;

    let stdout_tx = tx.clone();
    let stderr_tx = tx.clone();

    let stdout_task = tokio::spawn(forward_output(stdout, stdout_tx));
    let stderr_task = tokio::spawn(forward_output(stderr, stderr_tx));

    let status = tokio::select! {
        status = child.wait() => Some(status?),
        _ = &mut cancel => {
            terminate_process_group(&mut child).await?;
            None
        }
    };
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    let event = match status {
        None => BuildEvent::Cancelled,
        Some(status) if status.success() => BuildEvent::Finished(Ok(())),
        Some(status) => BuildEvent::Finished(Err(format!(
            "{} exited with status {}",
            command_name,
            status
                .code()
                .map_or_else(|| "<signal>".into(), |c| c.to_string())
        ))),
    };
    let _ = tx.send(event).await;
    Ok(())
}

async fn terminate_process_group(child: &mut Child) -> Result<()> {
    let Some(pid) = child.id() else {
        return Ok(());
    };
    let group = -(pid as i32);
    let term_result = unsafe { libc::kill(group, libc::SIGTERM) };
    if term_result == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("terminating rebuild process group");
        }
    }

    match timeout(Duration::from_secs(3), child.wait()).await {
        Ok(status) => {
            status.context("reaping cancelled rebuild")?;
        }
        Err(_) => {
            unsafe {
                libc::kill(group, libc::SIGKILL);
            }
            child.wait().await.context("reaping cancelled rebuild")?;
        }
    }
    Ok(())
}

async fn forward_output<R>(stream: R, tx: mpsc::Sender<BuildEvent>)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        let text = line.trim_end().to_string();
        if tx.send(BuildEvent::Line(text)).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BuildEvent, forward_output, nixos_rebuild_switch_cmd};
    use tokio::io::{AsyncWriteExt, duplex};
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn forwards_newline_delimited_lines() {
        let (mut writer, reader) = duplex(128);
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(forward_output(reader, tx));

        writer
            .write_all(b"first line\nsecond line\n")
            .await
            .unwrap();
        drop(writer);
        task.await.unwrap();

        assert!(matches!(rx.recv().await, Some(BuildEvent::Line(line)) if line == "first line"));
        assert!(matches!(rx.recv().await, Some(BuildEvent::Line(line)) if line == "second line"));
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn nixos_rebuild_uses_sudo_with_flake_ref() {
        let (cmd, args) = nixos_rebuild_switch_cmd(std::path::Path::new("/tmp/nixbox-config"));

        assert_eq!(cmd, "sudo");
        assert_eq!(args[0], "nixos-rebuild");
        assert_eq!(args[1], "switch");
        assert_eq!(args[2], "--flake");
        assert_eq!(args[3], "/tmp/nixbox-config#nixos");
    }

    #[tokio::test]
    async fn cancellation_stops_rebuild_and_emits_cancelled() {
        let (tx, mut rx) = mpsc::channel(8);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let task = tokio::spawn(super::rebuild(
            "sh",
            &["-c", "sleep 30"],
            &[],
            tx,
            cancel_rx,
        ));

        tokio::task::yield_now().await;
        cancel_tx.send(()).expect("send cancellation");

        let event = timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("rebuild should stop promptly")
            .expect("build event");
        assert!(matches!(event, BuildEvent::Cancelled));
        task.await.expect("join rebuild").expect("cancel rebuild");
    }
}
