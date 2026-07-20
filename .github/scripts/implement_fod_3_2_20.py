from pathlib import Path
import re


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text()
    if old not in text:
        raise SystemExit(f"missing literal in {path}: {old[:100]!r}")
    target.write_text(text.replace(old, new, 1))


def sub_once(path: str, pattern: str, replacement: str) -> None:
    target = Path(path)
    text = target.read_text()
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.S)
    if count != 1:
        raise SystemExit(f"pattern count in {path}: {count}: {pattern[:100]!r}")
    target.write_text(updated)


replace_once(
    "rust_indexer/src/hash.rs",
    'const HASH_ALGORITHM: &str = "sha256";',
    'pub(crate) const HASH_ALGORITHM: &str = "sha256";',
)
sub_once(
    "rust_indexer/src/hash.rs",
    r"fn compute_partial_hash\(path: &Path, snapshot: &FileSnapshot\) -> Result<Vec<u8>, String> \{.*?\n\}\n\nfn compute_full_hash\(path: &Path\) -> Result<Vec<u8>, String> \{.*?\n\}\n\npub fn rebuild_duplicate_sets",
    '''pub(crate) fn compute_partial_hash_from_file(
    file: &mut File,
    file_size: u64,
) -> Result<Vec<u8>, String> {
    let mut hasher = Sha256::new();
    if file_size == 0 {
        return Ok(hasher.finalize().to_vec());
    }
    for (offset, len) in sample_ranges(file_size) {
        let bytes = read_exact_range(file, offset, len)?;
        hasher.update(offset.to_le_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().to_vec())
}

fn compute_partial_hash(path: &Path, snapshot: &FileSnapshot) -> Result<Vec<u8>, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("unable to open {} for hashing: {err}", path.display()))?;
    compute_partial_hash_from_file(&mut file, snapshot.size)
}

pub(crate) fn compute_full_hash_from_file(file: &mut File) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|err| format!("seek failed before full hashing: {err}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; FULL_READ_BUFFER_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("read failed while hashing: {err}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_vec())
}

fn compute_full_hash(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("unable to open {} for hashing: {err}", path.display()))?;
    compute_full_hash_from_file(&mut file)
        .map_err(|err| format!("{err} for {}", path.display()))
}

pub fn rebuild_duplicate_sets''',
)

replace_once(
    "rust_indexer/Cargo.toml",
    'serde_json = "1"\nsha2 = "0.10"',
    'serde_json = "1"\nbase64 = "0.22"\nsha2 = "0.10"',
)

replace_once(
    "rust_indexer/src/cli.rs",
    "query the read-only file catalogue, register a filesystem-backed source",
    "query and read from the read-only file catalogue, register a filesystem-backed source",
)
replace_once(
    "rust_indexer/src/cli.rs",
    "  fod-indexer file show --id 42\\n  fod-indexer duplicate-set list",
    "  fod-indexer file show --id 42\\n  fod-indexer file read --id 42 --offset 0 --length 65536\\n  fod-indexer duplicate-set list",
)
sub_once(
    "rust_indexer/src/cli.rs",
    r"(    Show \{\n        #\[arg\(long\)\]\n        id: u64,\n    \},\n)(\}\n\n#\[derive\(Debug, Clone, Subcommand\)\]\npub enum SourceCommands)",
    r'''\1    #[command(
        about = "Read revalidated source bytes.",
        long_about = "Read a complete indexed source file or one byte range after revalidating its identity.\n\nThe command checks size, modification time, inode, device, and any stored partial or full hash before returning data. Missing, inaccessible, replaced, or changed files fail without returning bytes. Text output writes exact bytes to stdout and provenance to stderr; JSON output returns Base64 data and provenance."
    )]
    Read {
        #[arg(long)]
        id: u64,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[arg(long)]
        length: Option<u64>,
    },
\2''',
)
sub_once(
    "rust_indexer/src/cli.rs",
    r"(    #\[test\]\n    fn capabilities_is_not_treated_as_a_positional_source_command\(\) \{)",
    '''    #[test]
    fn parses_file_read_range() {
        let cli = Cli::try_parse_from([
            "fod-indexer",
            "--output",
            "json",
            "file",
            "read",
            "--id",
            "17",
            "--offset",
            "1024",
            "--length",
            "4096",
        ])
        .expect("file read command should parse");
        assert_eq!(cli.output, OutputFormat::Json);
        match cli.command {
            Commands::File {
                command: FileCommands::Read { id, offset, length },
            } => {
                assert_eq!(id, 17);
                assert_eq!(offset, 1024);
                assert_eq!(length, Some(4096));
            }
            _ => panic!("expected file read command"),
        }
    }

\1''',
)

replace_once(
    "rust_indexer/src/main.rs",
    "mod duplicate_set_api;\nmod hash;",
    "mod duplicate_set_api;\nmod file_read_api;\nmod hash;",
)
replace_once(
    "rust_indexer/src/main.rs",
    "use output::{print_json, SourceListOutput, SourceMutationOutput};\nuse std::path::Path;",
    "use output::{print_json, SourceListOutput, SourceMutationOutput};\nuse std::io::Write;\nuse std::path::Path;",
)
replace_once(
    "rust_indexer/src/main.rs",
    "let capabilities = duplicate_set_api::capabilities_output();",
    "let capabilities = file_read_api::capabilities_output();",
)
replace_once(
    "rust_indexer/src/main.rs",
    '''            FileCommands::Show { id } => {
                let file = read_api::show_file(&repo, id)?;
                if output.is_json() {
                    print_json(&file)?;
                } else {
                    println!("{}", file.human_readable());
                }
                Ok(())
            }
''',
    '''            FileCommands::Show { id } => {
                let file = read_api::show_file(&repo, id)?;
                if output.is_json() {
                    print_json(&file)?;
                } else {
                    println!("{}", file.human_readable());
                }
                Ok(())
            }
            FileCommands::Read { id, offset, length } => {
                let file = file_read_api::read_file(&repo, id, offset, length)?;
                if output.is_json() {
                    print_json(&file)?;
                } else {
                    eprintln!("{}", file.provenance_human_readable());
                    let mut stdout = std::io::stdout().lock();
                    stdout
                        .write_all(file.bytes())
                        .map_err(|err| format!("file_read_io_error: stdout write failed: {err}"))?;
                    stdout
                        .flush()
                        .map_err(|err| format!("file_read_io_error: stdout flush failed: {err}"))?;
                }
                Ok(())
            }
''',
)

replace_once("Cargo.toml", 'version = "3.2.19"', 'version = "3.2.20"')
Path("fod_version.txt").write_text("3.2.20\n")
