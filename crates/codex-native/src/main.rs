use codex_archive::AsarArchive;
use std::env;
use std::path::PathBuf;
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
        [command, web_root] if command == "run-shell" => shell::run(PathBuf::from(web_root)),
        _ => {
            let argv0 = env::args()
                .next()
                .map(PathBuf::from)
                .and_then(|path| path.file_name().map(|name| name.to_owned()))
                .unwrap_or_default();
            Err(format!(
                "usage:\n  {} list-asar <archive>\n  {} extract-asar <archive> <output-dir>\n  {} print-asar-file <archive> <path>\n  {} run-shell <web-root>",
                argv0.to_string_lossy(),
                argv0.to_string_lossy(),
                argv0.to_string_lossy(),
                argv0.to_string_lossy(),
            ))
        }
    }
}
