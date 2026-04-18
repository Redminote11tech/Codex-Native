use codex_archive::AsarArchive;
use std::env;
use std::path::{Path, PathBuf};
use std::process;

mod shell;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();

    match args.as_slice() {
        [] => shell::run(resolve_default_web_root()?),
        [command, archive] if command == "list-asar" => {
            let archive = AsarArchive::open(archive).map_err(|error| error.to_string())?;
            for file in archive.list_files() {
                println!("{file}");
            }
            Ok(())
        }
        [command, archive, output_dir] if command == "extract-asar" => {
            let archive = AsarArchive::open(archive).map_err(|error| error.to_string())?;
            archive
                .extract_all(output_dir)
                .map_err(|error| error.to_string())
        }
        [command, archive, file] if command == "print-asar-file" => {
            let archive = AsarArchive::open(archive).map_err(|error| error.to_string())?;
            let bytes = archive.read_file(file).map_err(|error| error.to_string())?;
            match std::str::from_utf8(bytes) {
                Ok(text) => {
                    print!("{text}");
                    Ok(())
                }
                Err(_) => Err(format!("asar entry is not valid utf-8: {file}")),
            }
        }
        [command] if command == "run-shell" => shell::run(resolve_default_web_root()?),
        [command, web_root] if command == "run-shell" => shell::run(PathBuf::from(web_root)),
        _ => {
            let argv0 = env::args()
                .next()
                .map(PathBuf::from)
                .and_then(|path| path.file_name().map(|name| name.to_owned()))
                .unwrap_or_default();
            Err(format!(
                "usage:\n  {} list-asar <archive>\n  {} extract-asar <archive> <output-dir>\n  {} print-asar-file <archive> <path>\n  {} run-shell [web-root]\n  {}",
                argv0.to_string_lossy(),
                argv0.to_string_lossy(),
                argv0.to_string_lossy(),
                argv0.to_string_lossy(),
                "If [web-root] is omitted, codex-native tries CODEX_NATIVE_WEB_ROOT, an installed /usr/share/codex-native/webview, or ./extracted/app-asar/webview.",
            ))
        }
    }
}

fn resolve_default_web_root() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();

    if let Some(value) = env::var_os("CODEX_NATIVE_WEB_ROOT") {
        candidates.push(PathBuf::from(value));
    }

    if let Ok(executable) = env::current_exe() {
        if let Some(bin_dir) = executable.parent() {
            candidates.push(bin_dir.join("../share/codex-native/webview"));
        }
    }

    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join("extracted/app-asar/webview"));
    }

    candidates.push(PathBuf::from("/usr/share/codex-native/webview"));
    candidates.push(PathBuf::from("/usr/local/share/codex-native/webview"));

    for candidate in candidates {
        if is_valid_web_root(&candidate) {
            return Ok(candidate);
        }
    }

    Err(
        "failed to resolve a frontend web root; pass `run-shell <web-root>` or set CODEX_NATIVE_WEB_ROOT"
            .to_string(),
    )
}

fn is_valid_web_root(path: &Path) -> bool {
    path.join("index.html").is_file()
}
