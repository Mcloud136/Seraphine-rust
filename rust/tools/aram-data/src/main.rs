//! ARAM balance snapshot generator (Rust port of scripts/update_aram_data.py).

mod builder;
mod fetch;
mod lua;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use builder::{build_from_balance, build_from_module, validate, Entry};
use fetch::Fetcher;
use lua::parse_module;

const SOURCE_WIKI: &str = "wiki.leagueoflegends.com Module:ChampionData/data";
const SOURCE_WIKI_FALLBACK: &str =
    "wiki.leagueoflegends.com Module:ChampionData/data (bootstrap via jsdelivr)";
const SOURCE_BOOTSTRAP: &str = "wiki.leagueoflegends.com Module:ChampionData/data (bootstrap)";

struct Args {
    source: String,
    balance_json: Option<PathBuf>,
    version: Option<String>,
    output_dir: PathBuf,
    names_url: String,
    no_names: bool,
    dry_run: bool,
    wiki_file: Option<PathBuf>,
    generated_at: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        source: "auto".into(),
        balance_json: None,
        version: None,
        output_dir: PathBuf::from("rust/resources/data"),
        names_url: fetch::HERO_LIST_URL.into(),
        no_names: false,
        dry_run: false,
        wiki_file: None,
        generated_at: None,
    };
    let mut items = std::env::args().skip(1);
    while let Some(item) = items.next() {
        match item.as_str() {
            "--source" => {
                let value = items.next().ok_or("--source needs a value")?;
                if !matches!(value.as_str(), "auto" | "wiki" | "balance-json") {
                    return Err(format!("invalid source: {value}"));
                }
                args.source = value;
            }
            "--balance-json" => {
                args.balance_json = Some(PathBuf::from(
                    items.next().ok_or("--balance-json needs a path")?,
                ));
            }
            "--version" => args.version = Some(items.next().ok_or("--version needs a value")?),
            "--output-dir" => {
                args.output_dir = PathBuf::from(items.next().ok_or("--output-dir needs a path")?);
            }
            "--names-url" => args.names_url = items.next().ok_or("--names-url needs a value")?,
            "--no-names" => args.no_names = true,
            "--dry-run" => args.dry_run = true,
            "--wiki-file" => {
                args.wiki_file = Some(PathBuf::from(
                    items.next().ok_or("--wiki-file needs a path")?,
                ));
            }
            "--generated-at" => {
                args.generated_at = Some(items.next().ok_or("--generated-at needs a value")?);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if args.source == "balance-json" && args.balance_json.is_none() {
        return Err("--balance-json is required for --source balance-json".into());
    }
    Ok(args)
}

fn profile(champions: &BTreeMap<u64, Entry>) -> Map<String, Value> {
    let mut entries = Map::new();
    for (id, entry) in champions {
        let mut fields = Map::new();
        for (name, value) in entry {
            fields.insert(name.clone(), Value::String(value.clone()));
        }
        entries.insert(id.to_string(), Value::Object(fields));
    }
    entries
}

async fn wiki_script(fetcher: &Fetcher, args: &Args) -> Result<String, String> {
    match &args.wiki_file {
        Some(path) => {
            std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
        }
        None => fetcher.wiki_script().await,
    }
}

async fn generate(args: &Args) -> Result<Value, String> {
    let fetcher = Fetcher::new()?;
    let names = if args.no_names {
        HashMap::new()
    } else {
        fetcher.champion_names(&args.names_url).await
    };

    let (champions, version, source) = match args.source.as_str() {
        "balance-json" => {
            let path = args.balance_json.as_ref().ok_or("missing balance json")?;
            let text = std::fs::read_to_string(path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let balance: Value =
                serde_json::from_str(&text).map_err(|error| format!("balance json: {error}"))?;
            let champions = build_from_balance(&balance, &names);
            let version = args.version.clone().or_else(|| {
                balance
                    .get("_patch")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            (champions, version, SOURCE_BOOTSTRAP.to_string())
        }
        _ => match wiki_script(&fetcher, args).await {
            Ok(script) => {
                let module = parse_module(&script)?;
                let champions = build_from_module(&module, &names);
                let version = match args.version.clone() {
                    Some(version) => Some(version),
                    None => Some(fetcher.latest_version().await?),
                };
                (champions, version, SOURCE_WIKI.to_string())
            }
            Err(error) if args.source == "auto" => {
                eprintln!("warning: wiki source unavailable ({error}); falling back to the bootstrap snapshot");
                let balance = fetcher.bootstrap().await?;
                let champions = build_from_balance(&balance, &names);
                let version = match args.version.clone() {
                    Some(version) => Some(version),
                    None => balance
                        .get("_patch")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                };
                (champions, version, SOURCE_WIKI_FALLBACK.to_string())
            }
            Err(error) => return Err(error),
        },
    };

    let version = version.ok_or_else(|| "no usable version (pass --version)".to_string())?;
    validate(&version, &champions)?;
    let game_version = version.split('.').take(2).collect::<Vec<_>>().join(".");
    let generated_at = args.generated_at.clone().unwrap_or_else(utc_now);
    let champions_value = Value::Object(profile(&champions));

    let mut data = Map::new();
    data.insert("schema".into(), json!(1));
    data.insert("version".into(), Value::String(version.clone()));
    data.insert("gameVersion".into(), Value::String(game_version.clone()));
    data.insert("generatedAt".into(), Value::String(generated_at.clone()));
    data.insert("source".into(), Value::String(source.clone()));
    data.insert("champions".into(), champions_value);
    Ok(Value::Object(data))
}

fn write_snapshot(output_dir: &Path, data: &Value, dry_run: bool) -> Result<Value, String> {
    let data_bytes =
        serde_json::to_vec(data).map_err(|error| format!("serialize data: {error}"))?;
    let mut manifest = Map::new();
    manifest.insert("schema".into(), json!(1));
    manifest.insert(
        "version".into(),
        data.get("version").cloned().unwrap_or(Value::Null),
    );
    manifest.insert(
        "gameVersion".into(),
        data.get("gameVersion").cloned().unwrap_or(Value::Null),
    );
    manifest.insert(
        "generatedAt".into(),
        data.get("generatedAt").cloned().unwrap_or(Value::Null),
    );
    manifest.insert("file".into(), Value::String("AramBuff.json".into()));
    manifest.insert("size".into(), json!(data_bytes.len()));
    manifest.insert(
        "sha256".into(),
        Value::String(format!("{:x}", Sha256::digest(&data_bytes))),
    );
    let champions = data
        .get("champions")
        .and_then(Value::as_object)
        .map(Map::len)
        .unwrap_or(0);
    manifest.insert("champions".into(), json!(champions));
    manifest.insert(
        "source".into(),
        data.get("source").cloned().unwrap_or(Value::Null),
    );
    let manifest = Value::Object(manifest);
    if dry_run {
        return Ok(json!({ "manifest": manifest, "dryRun": true }));
    }
    let manifest_text = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("serialize manifest: {error}"))?;
    write_atomic(&output_dir.join("AramBuff.json"), &data_bytes)?;
    write_atomic(
        &output_dir.join("aram-manifest.json"),
        format!("{manifest_text}\n").as_bytes(),
    )?;
    Ok(manifest)
}

fn write_atomic(path: &Path, payload: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    std::fs::write(&temporary, payload)
        .map_err(|error| format!("{}: {error}", temporary.display()))?;
    #[cfg(windows)]
    {
        if path.exists() {
            std::fs::remove_file(path).map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {error}", path.display())
    })
}

fn utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = rest / 3_600;
    let minute = (rest % 3_600) / 60;
    let second = rest % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    let data = generate(&args).await?;
    let manifest = write_snapshot(&args.output_dir, &data, args.dry_run)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&manifest)
            .map_err(|error| format!("serialize result: {error}"))?
    );
    Ok(())
}
