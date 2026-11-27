use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use reqwest::blocking::{multipart, Client};
use reqwest::StatusCode;
use serde_json::{json, Value};

const API_ROOT: &str = "https://gitee.com/api/v5";

#[derive(Parser)]
#[command(name = "release-tool", about = "Release metadata and Gitee publishing")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a Tauri updater manifest from a signature file.
    Manifest {
        #[arg(long)]
        signature: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long)]
        url: String,
        #[arg(long, default_value = "")]
        notes: String,
        #[arg(long)]
        pub_date: String,
        #[arg(long, default_value = "windows-x86_64")]
        platform: String,
        #[arg(long, default_value = "latest.json")]
        output: PathBuf,
    },
    /// Replace the installer URL in an existing updater manifest.
    RewriteManifest {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        url: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Print the version from tauri.conf.json.
    Version {
        #[arg(long)]
        config: PathBuf,
    },
    /// Print a short ARAM snapshot report.
    AramReport {
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Create/reuse a Gitee release and replace the named attachments.
    Gitee {
        #[arg(long)]
        owner: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        tag: String,
        #[arg(long, default_value = "Release")]
        release_name: String,
        #[arg(long, default_value = "")]
        release_body: String,
        #[arg(long, default_value = "main")]
        target: String,
        #[arg(long, required = true)]
        files: Vec<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Manifest {
            signature,
            version,
            url,
            notes,
            pub_date,
            platform,
            output,
        } => {
            let signature = fs::read_to_string(&signature)
                .with_context(|| format!("read signature {}", signature.display()))?;
            let manifest = build_manifest(&signature, &version, &url, &notes, &pub_date, &platform);
            write_json(&output, &manifest)?;
            println!("wrote {} version={version}", output.display());
        }
        Command::RewriteManifest { input, url, output } => {
            let bytes = fs::read(&input).with_context(|| format!("read {}", input.display()))?;
            let mut manifest: Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse {}", input.display()))?;
            rewrite_manifest(&mut manifest, &url)?;
            write_json(&output, &manifest)?;
            println!("wrote {}", output.display());
        }
        Command::Version { config } => {
            let version = read_version(&config)?;
            println!("{version}");
        }
        Command::AramReport { manifest } => {
            let bytes =
                fs::read(&manifest).with_context(|| format!("read {}", manifest.display()))?;
            let value: Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse {}", manifest.display()))?;
            println!(
                "version={} champions={} sha256={}",
                value.get("version").and_then(Value::as_str).unwrap_or("?"),
                value.get("champions").and_then(Value::as_u64).unwrap_or(0),
                value.get("sha256").and_then(Value::as_str).unwrap_or("?")
            );
        }
        Command::Gitee {
            owner,
            repo,
            tag,
            release_name,
            release_body,
            target,
            files,
        } => {
            publish_gitee(
                &owner,
                &repo,
                &tag,
                &release_name,
                &release_body,
                &target,
                &files,
            )?;
        }
    }
    Ok(())
}

fn build_manifest(
    signature: &str,
    version: &str,
    url: &str,
    notes: &str,
    pub_date: &str,
    platform: &str,
) -> Value {
    json!({
        "version": version,
        "notes": notes,
        "pub_date": pub_date,
        "platforms": {
            platform: {
                "signature": signature.trim(),
                "url": url,
            }
        }
    })
}

fn rewrite_manifest(manifest: &mut Value, url: &str) -> Result<()> {
    let platform = manifest
        .get_mut("platforms")
        .and_then(Value::as_object_mut)
        .and_then(|platforms| platforms.get_mut("windows-x86_64"))
        .and_then(Value::as_object_mut)
        .context("manifest has no windows-x86_64 platform")?;
    platform.insert("url".into(), Value::String(url.into()));
    Ok(())
}

fn read_version(config: &Path) -> Result<String> {
    let bytes = fs::read(config).with_context(|| format!("read {}", config.display()))?;
    let value: Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", config.display()))?;
    value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("tauri.conf.json has no version")
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
    }
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

fn publish_gitee(
    owner: &str,
    repo: &str,
    tag: &str,
    release_name: &str,
    release_body: &str,
    target: &str,
    files: &[PathBuf],
) -> Result<()> {
    let token = std::env::var("GITEE_TOKEN")
        .context("GITEE_TOKEN is not set")?
        .trim()
        .to_owned();
    if token.is_empty() {
        bail!("GITEE_TOKEN is empty");
    }
    for path in files {
        if !path.is_file() {
            bail!("missing file: {}", path.display());
        }
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .context("build HTTP client")?;
    let base = format!("{API_ROOT}/repos/{owner}/{repo}");
    let release = ensure_release(
        &client,
        &base,
        &token,
        tag,
        release_name,
        release_body,
        target,
    )?;
    let release_id = release
        .get("id")
        .and_then(Value::as_i64)
        .context("Gitee release has no id")?;
    let wanted: Vec<&str> = files
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect();

    for attachment in list_attachments(&client, &base, release_id, &token)? {
        let Some(name) = attachment.get("name").and_then(Value::as_str) else {
            continue;
        };
        if wanted.contains(&name) {
            if let Some(id) = attachment.get("id").and_then(Value::as_i64) {
                delete_attachment(&client, &base, release_id, id, &token)?;
            }
        }
    }

    let tag_name = release
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or(tag);
    for path in files {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .with_context(|| format!("invalid filename {}", path.display()))?;
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        upload_with_retry(&client, &base, release_id, &token, name, bytes)?;
        println!("https://gitee.com/{owner}/{repo}/releases/download/{tag_name}/{name}");
    }
    Ok(())
}

fn ensure_release(
    client: &Client,
    base: &str,
    token: &str,
    tag: &str,
    name: &str,
    body: &str,
    target: &str,
) -> Result<Value> {
    let url = format!("{base}/releases/tags/{tag}");
    let response = client
        .get(&url)
        .query(&[("access_token", token)])
        .send()
        .context("GET Gitee release")?;
    if response.status() == StatusCode::NOT_FOUND {
        let response = client
            .post(format!("{base}/releases"))
            .form(&[
                ("access_token", token),
                ("tag_name", tag),
                ("name", name),
                ("body", body),
                ("target_commitish", target),
            ])
            .send()
            .context("POST Gitee release")?;
        return parse_response(response, "create Gitee release");
    }
    parse_response(response, "GET Gitee release")
}

fn list_attachments(
    client: &Client,
    base: &str,
    release_id: i64,
    token: &str,
) -> Result<Vec<Value>> {
    let response = client
        .get(format!("{base}/releases/{release_id}/attach_files"))
        .query(&[("access_token", token), ("per_page", "100")])
        .send()
        .context("GET Gitee attachments")?;
    let value = parse_response(response, "GET Gitee attachments")?;
    Ok(value.as_array().cloned().unwrap_or_default())
}

fn delete_attachment(
    client: &Client,
    base: &str,
    release_id: i64,
    attachment_id: i64,
    token: &str,
) -> Result<()> {
    let response = client
        .delete(format!(
            "{base}/releases/{release_id}/attach_files/{attachment_id}"
        ))
        .query(&[("access_token", token)])
        .send()
        .context("DELETE Gitee attachment")?;
    if response.status() >= StatusCode::BAD_REQUEST {
        bail!(
            "delete attachment {attachment_id} failed: {}",
            response_text(response)
        );
    }
    Ok(())
}

fn upload_with_retry(
    client: &Client,
    base: &str,
    release_id: i64,
    token: &str,
    name: &str,
    bytes: Vec<u8>,
) -> Result<()> {
    let mut last_error = None;
    for attempt in 1..=3 {
        let form = multipart::Form::new()
            .text("access_token", token.to_owned())
            .part(
                "file",
                multipart::Part::bytes(bytes.clone()).file_name(name.to_owned()),
            );
        match client
            .post(format!("{base}/releases/{release_id}/attach_files"))
            .multipart(form)
            .send()
        {
            Ok(response) if response.status() < StatusCode::BAD_REQUEST => return Ok(()),
            Ok(response) => {
                last_error = Some(format!(
                    "upload {name} attempt {attempt}: {}",
                    response_text(response)
                ));
            }
            Err(error) => {
                last_error = Some(format!("upload {name} attempt {attempt}: {error}"));
            }
        }
        for attachment in list_attachments(client, base, release_id, token).unwrap_or_default() {
            if attachment.get("name").and_then(Value::as_str) == Some(name) {
                if let Some(id) = attachment.get("id").and_then(Value::as_i64) {
                    let _ = delete_attachment(client, base, release_id, id, token);
                }
            }
        }
        if attempt < 3 {
            sleep(Duration::from_secs(5 * attempt));
        }
    }
    bail!(last_error.unwrap_or_else(|| format!("upload {name} failed")))
}

fn parse_response(response: reqwest::blocking::Response, action: &str) -> Result<Value> {
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if status >= StatusCode::BAD_REQUEST {
        bail!("{action} failed: HTTP {status}: {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("{action}: invalid JSON: {text}"))
}

fn response_text(response: reqwest::blocking::Response) -> String {
    let status = response.status();
    let text = response.text().unwrap_or_default();
    format!(
        "HTTP {status}: {}",
        text.chars().take(300).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_signature_and_url() {
        let manifest = build_manifest(
            "sig\n",
            "2.0.2",
            "https://example.com/a.exe",
            "notes",
            "2026-11-27T00:00:00Z",
            "windows-x86_64",
        );
        let platform = &manifest["platforms"]["windows-x86_64"];
        assert_eq!(platform["signature"], "sig");
        assert_eq!(platform["url"], "https://example.com/a.exe");
        assert_eq!(manifest["version"], "2.0.2");
    }

    #[test]
    fn rewrite_manifest_replaces_url() {
        let mut manifest = json!({
            "version": "2.0.2",
            "platforms": { "windows-x86_64": { "signature": "sig", "url": "old" } }
        });
        rewrite_manifest(&mut manifest, "new").expect("rewrite");
        assert_eq!(
            manifest["platforms"]["windows-x86_64"]["url"],
            Value::String("new".into())
        );
    }
}
