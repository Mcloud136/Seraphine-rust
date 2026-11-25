#![windows_subsystem = "windows"]

use seraphine_platform::{atomic_replace_file, list_processes};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone)]
struct UpdateFile {
    path: PathBuf,
    size: u64,
    sha256: String,
}

fn safe_relative(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.contains('\0') || value.contains('\\') {
        return Err("invalid manifest path".to_string());
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("manifest path escapes package".to_string());
    }
    Ok(path.to_path_buf())
}

fn sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn atomic_copy(source: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "destination parent is missing".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = destination.with_extension(format!(
        "{}.new",
        destination
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("tmp")
    ));
    fs::copy(source, &temporary).map_err(|error| error.to_string())?;
    atomic_replace_file(&temporary, destination).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        error.to_string()
    })
}

fn write_status(path: &Path, status: &Value) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(payload) = serde_json::to_vec(status) {
        let temporary = path.with_extension("json.tmp");
        if fs::write(&temporary, payload).is_ok() {
            let _ = atomic_replace_file(&temporary, path);
        }
    }
}

fn wait_for_processes(names: &[&str], timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let running = list_processes()
            .map_err(|error| error.to_string())?
            .into_iter()
            .any(|process| {
                names
                    .iter()
                    .any(|name| process.name.eq_ignore_ascii_case(name))
            });
        if !running {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for Seraphine to exit".to_string());
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn wait_for_pid(pid: u32, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let running = list_processes()
            .map_err(|error| error.to_string())?
            .into_iter()
            .any(|process| process.pid == pid);
        if !running {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for updater to exit".to_string());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn read_manifest(source: &Path) -> Result<(String, Vec<UpdateFile>), String> {
    let manifest_path = source.join("update-manifest.json");
    let length = fs::metadata(&manifest_path)
        .map_err(|error| format!("update manifest is missing: {error}"))?
        .len();
    if length == 0 || length > MAX_MANIFEST_BYTES {
        return Err("update manifest size is invalid".to_string());
    }
    let bytes = fs::read(manifest_path).map_err(|error| error.to_string())?;
    let manifest: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "update version is missing".to_string())?
        .to_string();
    let entries = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "update files are missing".to_string())?;
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in entries {
        let relative = safe_relative(
            entry
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "update file path is missing".to_string())?,
        )?;
        if !seen.insert(relative.clone()) {
            return Err("duplicate update file".to_string());
        }
        let size = entry
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| "update file size is missing".to_string())?;
        let hash = entry
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| "update file hash is invalid".to_string())?
            .to_ascii_lowercase();
        let source_file = source.join(&relative);
        let canonical = fs::canonicalize(&source_file)
            .map_err(|error| format!("{}: {error}", source_file.display()))?;
        if !canonical.starts_with(fs::canonicalize(source).map_err(|error| error.to_string())?) {
            return Err("update file escapes source directory".to_string());
        }
        if fs::metadata(&canonical)
            .map_err(|error| error.to_string())?
            .len()
            != size
            || sha256(&canonical)? != hash
        {
            return Err(format!("update checksum failed: {}", relative.display()));
        }
        files.push(UpdateFile {
            path: relative,
            size,
            sha256: hash,
        });
    }
    if files.is_empty() {
        return Err("update contains no files".to_string());
    }
    Ok((version, files))
}

fn read_old_files(target: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let path = target.join("filelist.txt");
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => return Err(error.to_string()),
    };
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| safe_relative(&line.replace('\\', "/")))
        .collect()
}

fn cleanup_stale_finalizers() {
    let directory = env::temp_dir().join("Seraphine").join("Updater");
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let cutoff = SystemTime::now() - Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("seraphine-updater-finalize-"))
        {
            continue;
        }
        if entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified < cutoff)
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn launch(app: &Path) -> Result<(), String> {
    Command::new(app)
        .current_dir(
            app.parent()
                .ok_or_else(|| "app parent is missing".to_string())?,
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn install_update(
    source: &Path,
    target: &Path,
    status_path: &Path,
    no_launch: bool,
) -> Result<(), String> {
    let data_root = env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("Seraphine");
    install_update_with_backup(
        source,
        target,
        status_path,
        no_launch,
        &data_root.join("UpdateBackup"),
    )
}

fn install_update_with_backup(
    source: &Path,
    target: &Path,
    status_path: &Path,
    no_launch: bool,
    backup_root: &Path,
) -> Result<(), String> {
    cleanup_stale_finalizers();
    wait_for_processes(
        &["Seraphine.exe", "seraphine-sidecar.exe"],
        Duration::from_secs(30),
    )?;
    let (version, files) = read_manifest(source)?;
    let target = fs::canonicalize(target).map_err(|error| error.to_string())?;
    let old_files = read_old_files(&target)?;
    let new_files = files
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<BTreeSet<_>>();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let backup = backup_root.join(format!("{version}-{timestamp}"));
    fs::create_dir_all(&backup).map_err(|error| error.to_string())?;
    let current_exe = fs::canonicalize(env::current_exe().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let mut installed = Vec::new();
    let mut removed = Vec::new();
    let mut self_update = None;
    let result = (|| {
        for file in &files {
            let relative = &file.path;
            let destination = target.join(relative);
            let source_file = source.join(relative);
            if destination == current_exe {
                let staged = destination.with_extension("exe.new");
                atomic_copy(&source_file, &staged)?;
                if sha256(&staged)? != file.sha256 {
                    let _ = fs::remove_file(&staged);
                    return Err("staged updater checksum failed".to_string());
                }
                self_update = Some((staged, destination));
                continue;
            }
            if destination.is_file() {
                let backup_file = backup.join(relative);
                if let Some(parent) = backup_file.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::copy(&destination, &backup_file).map_err(|error| error.to_string())?;
            }
            atomic_copy(&source_file, &destination)?;
            if fs::metadata(&destination)
                .map_err(|error| error.to_string())?
                .len()
                != file.size
                || sha256(&destination)? != file.sha256
            {
                return Err(format!("installed checksum failed: {}", relative.display()));
            }
            installed.push(destination);
        }
        let manifest_destination = target.join("update-manifest.json");
        if manifest_destination.is_file() {
            fs::copy(&manifest_destination, backup.join("update-manifest.json"))
                .map_err(|error| error.to_string())?;
        }
        atomic_copy(&source.join("update-manifest.json"), &manifest_destination)?;
        installed.push(manifest_destination);
        for relative in old_files.difference(&new_files) {
            let destination = target.join(relative);
            if !destination.is_file() {
                continue;
            }
            let backup_file = backup.join(relative);
            if let Some(parent) = backup_file.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::copy(&destination, &backup_file).map_err(|error| error.to_string())?;
            fs::remove_file(&destination).map_err(|error| error.to_string())?;
            removed.push(destination);
        }
        Ok(())
    })();
    if let Err(error) = result {
        for path in installed.iter().rev() {
            let _ = fs::remove_file(path);
        }
        for path in removed.iter().rev() {
            let _ = fs::remove_file(path);
        }
        for entry in walk_files(&backup) {
            if let Ok(relative) = entry.strip_prefix(&backup) {
                let _ = atomic_copy(&entry, &target.join(relative));
            }
        }
        if let Some((staged, _)) = self_update {
            let _ = fs::remove_file(staged);
        }
        return Err(error);
    }

    if let Some((staged, self_target)) = self_update {
        write_status(
            status_path,
            &json!({"status": "finalizing", "version": version, "backup": backup}),
        );
        if no_launch {
            return Ok(());
        }
        let app = target.join("Seraphine.exe");
        let finalizer_dir = env::temp_dir().join("Seraphine").join("Updater");
        fs::create_dir_all(&finalizer_dir).map_err(|error| error.to_string())?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let finalizer = finalizer_dir.join(format!(
            "seraphine-updater-finalize-{}-{nonce}.exe",
            std::process::id()
        ));
        fs::copy(
            env::current_exe().map_err(|error| error.to_string())?,
            &finalizer,
        )
        .map_err(|error| error.to_string())?;
        let spawn_result = Command::new(&finalizer)
            .arg("--finalize")
            .arg("--parent")
            .arg(std::process::id().to_string())
            .arg("--staged")
            .arg(staged)
            .arg("--target")
            .arg(self_target)
            .arg("--app")
            .arg(app)
            .arg("--status")
            .arg(status_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Err(error) = spawn_result {
            let _ = fs::remove_file(finalizer);
            return Err(error.to_string());
        }
        return Ok(());
    }
    write_status(
        status_path,
        &json!({"status": "completed", "version": version, "backup": backup}),
    );
    if !no_launch {
        launch(&target.join("Seraphine.exe"))?;
    }
    Ok(())
}

fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

fn finalize(
    parent: u32,
    staged: &Path,
    target: &Path,
    app: &Path,
    status: &Path,
) -> Result<(), String> {
    wait_for_pid(parent, Duration::from_secs(30))?;
    atomic_replace_file(staged, target).map_err(|error| error.to_string())?;
    write_status(status, &json!({"status": "completed", "backupKept": true}));
    launch(app)
}

fn run() -> Result<(), String> {
    let args = env::args().collect::<Vec<_>>();
    let status_default = env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("Seraphine")
        .join("update-status.json");
    let mut status_path = status_default;
    let mut source = None;
    let mut target = None;
    let mut no_launch = false;
    let mut finalize_args = None;
    let mut index = 1;
    while index < args.len() {
        let value = args.get(index + 1).map(String::as_str);
        match args[index].as_str() {
            "--source" => source = value.map(PathBuf::from),
            "--target" => target = value.map(PathBuf::from),
            "--status" => {
                status_path = value
                    .map(PathBuf::from)
                    .unwrap_or_else(|| status_path.clone())
            }
            "--no-launch" => {
                no_launch = true;
                index -= 1;
            }
            "--finalize" => {
                let mut parent = None;
                let mut staged = None;
                let mut final_target = None;
                let mut app = None;
                let mut final_status = status_path.clone();
                index += 1;
                while index < args.len() {
                    let next = args.get(index + 1).map(String::as_str);
                    match args[index].as_str() {
                        "--parent" => parent = next.and_then(|value| value.parse().ok()),
                        "--staged" => staged = next.map(PathBuf::from),
                        "--target" => final_target = next.map(PathBuf::from),
                        "--app" => app = next.map(PathBuf::from),
                        "--status" => {
                            final_status = next.map(PathBuf::from).unwrap_or(final_status)
                        }
                        _ => return Err("invalid finalize argument".to_string()),
                    }
                    index += 2;
                }
                finalize_args = Some((
                    parent.ok_or_else(|| "parent pid is missing".to_string())?,
                    staged.ok_or_else(|| "staged updater is missing".to_string())?,
                    final_target.ok_or_else(|| "updater target is missing".to_string())?,
                    app.ok_or_else(|| "app path is missing".to_string())?,
                    final_status,
                ));
                break;
            }
            _ => return Err("unknown updater argument".to_string()),
        }
        index += 2;
    }
    if let Some((parent, staged, target, app, status)) = finalize_args {
        return finalize(parent, &staged, &target, &app, &status);
    }
    let source = source.ok_or_else(|| "source directory is missing".to_string())?;
    let target = target.ok_or_else(|| "target directory is missing".to_string())?;
    install_update(&source, &target, &status_path, no_launch)
}

fn main() {
    let status_path = env::args()
        .position(|arg| arg == "--status")
        .and_then(|index| env::args().nth(index + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(env::temp_dir)
                .join("Seraphine")
                .join("update-status.json")
        });
    if let Err(error) = run() {
        write_status(&status_path, &json!({"status": "failed", "error": error}));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_directory(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "seraphine-updater-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_manifest(root: &Path, files: &[(&str, &[u8])]) {
        fs::create_dir_all(root).unwrap();
        let entries = files
            .iter()
            .map(|(path, content)| {
                let destination = root.join(path);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::write(destination, content).unwrap();
                json!({
                    "path": path,
                    "size": content.len(),
                    "sha256": format!("{:x}", Sha256::digest(content))
                })
            })
            .collect::<Vec<_>>();
        fs::write(
            root.join("update-manifest.json"),
            serde_json::to_vec(&json!({"version": "test", "files": entries})).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn rejects_unsafe_manifest_paths() {
        assert!(safe_relative("../Seraphine.exe").is_err());
        assert!(safe_relative("/Seraphine.exe").is_err());
        assert!(safe_relative("a\\b.exe").is_err());
        assert_eq!(
            safe_relative("_internal/app.py").unwrap(),
            Path::new("_internal/app.py")
        );
    }

    #[test]
    fn installs_verified_files_and_removes_legacy_files() {
        let source = unique_directory("source");
        let target = unique_directory("target");
        let status = unique_directory("status.json");
        fs::create_dir_all(target.join("_internal")).unwrap();
        fs::write(target.join("Seraphine.exe"), b"old").unwrap();
        fs::write(target.join("_internal/old.py"), b"legacy").unwrap();
        fs::write(
            target.join("filelist.txt"),
            b"Seraphine.exe\r\n_internal/old.py\r\n",
        )
        .unwrap();
        write_manifest(
            &source,
            &[("Seraphine.exe", b"new"), ("_internal/new.py", b"new-py")],
        );
        let backup = unique_directory("backup");
        install_update_with_backup(&source, &target, &status, true, &backup).unwrap();
        assert_eq!(fs::read(target.join("Seraphine.exe")).unwrap(), b"new");
        assert_eq!(
            fs::read(target.join("_internal/new.py")).unwrap(),
            b"new-py"
        );
        assert!(!target.join("_internal/old.py").exists());
        assert!(target.join("update-manifest.json").is_file());
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(target).unwrap();
        fs::remove_dir_all(backup).unwrap();
    }

    #[test]
    fn rolls_back_installed_files_when_replacement_fails() {
        let source = unique_directory("rollback-source");
        let target = unique_directory("rollback-target");
        let backup = unique_directory("rollback-backup");
        let status = unique_directory("rollback-status.json");
        fs::create_dir_all(target.join("_internal/new.py")).unwrap();
        fs::write(target.join("Seraphine.exe"), b"old").unwrap();
        write_manifest(
            &source,
            &[("Seraphine.exe", b"new"), ("_internal/new.py", b"blocked")],
        );
        assert!(install_update_with_backup(&source, &target, &status, true, &backup).is_err());
        assert_eq!(fs::read(target.join("Seraphine.exe")).unwrap(), b"old");
        assert!(target.join("_internal/new.py").is_dir());
        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(target).unwrap();
        fs::remove_dir_all(backup).unwrap();
    }

    #[test]
    fn rejects_checksum_mismatch_before_install() {
        let source = unique_directory("bad-source");
        write_manifest(&source, &[("Seraphine.exe", b"new")]);
        let manifest_path = source.join("update-manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["files"][0]["sha256"] = json!("0".repeat(64));
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(read_manifest(&source).is_err());
        fs::remove_dir_all(source).unwrap();
    }
}
