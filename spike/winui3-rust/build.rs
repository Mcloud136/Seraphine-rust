use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use zip::ZipArchive;

const RUNTIME_PACKAGE: &str = "Microsoft.WindowsAppSDK.Runtime";
const RUNTIME_VERSION: &str = "2.4.0";
const WEBVIEW_PACKAGE: &str = "Microsoft.Web.WebView2";
const WEBVIEW_VERSION: &str = "1.0.4078.44";
const NUGET_URL: &str = "https://www.nuget.org/api/v2/package";

fn cache_root() -> PathBuf {
    let base = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    base.join("windows-reactor-setup").join("temp")
}

fn download(url: &str, destination: &Path) {
    if destination.is_file() {
        return;
    }

    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let status = Command::new(
        PathBuf::from(env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("curl.exe"),
    )
    .args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--output",
    ])
    .arg(destination)
    .arg(url)
    .status()
    .unwrap();
    assert!(status.success(), "failed to download {url}");
}

fn extract_zip(archive: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    let file = fs::File::open(archive).unwrap();
    let mut zip = ZipArchive::new(file).unwrap();
    zip.extract(destination).unwrap();
}

fn ensure_package(name: &str, version: &str) -> PathBuf {
    let root = cache_root();
    let package = root.join(format!("{name}.{version}.nupkg"));
    let extracted = root.join(format!("{name}-{version}"));

    if !extracted.is_dir() {
        download(&format!("{NUGET_URL}/{name}/{version}"), &package);
        let staging = extracted.with_file_name(format!("{name}-{version}.partial"));
        if staging.exists() {
            fs::remove_dir_all(&staging).unwrap();
        }
        extract_zip(&package, &staging);
        fs::rename(staging, &extracted).unwrap();
    }

    extracted
}

fn prepare_runtime() {
    let runtime = ensure_package(RUNTIME_PACKAGE, RUNTIME_VERSION);
    let expected_msix = runtime.join("MSIX");
    if !expected_msix.exists() {
        fs::rename(runtime.join("tools").join("MSIX"), expected_msix).unwrap();
    }

    let extracted = runtime.join(".msix_extract");
    if !extracted.is_dir() {
        extract_zip(
            &runtime.join("MSIX/win10-x64/Microsoft.WindowsAppRuntime.2.msix"),
            &extracted,
        );
    }

    let webview = ensure_package(WEBVIEW_PACKAGE, WEBVIEW_VERSION);
    let expected_webview = webview.join("win-x64");
    if !expected_webview.exists() {
        fs::rename(webview.join("runtimes").join("win-x64"), expected_webview).unwrap();
    }
}

fn prepare_private_system_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let private_root = manifest_dir.join("target/.windows-reactor-tools");
    let system32 = private_root.join("System32");
    fs::create_dir_all(&system32).unwrap();

    let source = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap()
        .join("System32")
        .join("curl.exe");
    fs::copy(source, system32.join("curl.exe")).unwrap();
    private_root
}

fn main() {
    prepare_runtime();
    let private_root = prepare_private_system_root();

    // The build script is single-threaded and has not spawned any child processes yet.
    unsafe { env::set_var("SystemRoot", private_root) };
    windows_reactor_setup::as_self_contained();
}
