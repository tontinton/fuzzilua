use std::fs;
use std::path::Path;

use tracing::warn;

use crate::CorpusEntry;
use crate::error::CorpusError;

const ENTRY_PREFIX: &str = "entry_";
const ENTRY_EXT: &str = ".bin";

pub fn save_entry(dir: &Path, entry: &CorpusEntry) -> Result<(), CorpusError> {
    fs::create_dir_all(dir)?;
    let data = bincode::serialize(entry)?;
    let hash = fnv1a_64(&data);
    let final_path = dir.join(format!("{ENTRY_PREFIX}{hash:016x}{ENTRY_EXT}"));

    if final_path.exists() {
        return Ok(());
    }

    let tmp_path = dir.join(format!(".tmp_{hash:016x}{ENTRY_EXT}"));
    fs::write(&tmp_path, &data)?;
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

pub fn remove_entries(dir: &Path) -> Result<(), CorpusError> {
    if !dir.exists() {
        return Ok(());
    }
    for path in list_entry_paths(dir)? {
        fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn load_entries(dir: &Path) -> Result<Vec<CorpusEntry>, CorpusError> {
    let mut entries = Vec::new();
    for path in list_entry_paths(dir)? {
        let data = fs::read(&path)?;
        match bincode::deserialize::<CorpusEntry>(&data) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                warn!(path = %path.display(), "skipping corrupt corpus entry: {e}");
            }
        }
    }
    Ok(entries)
}

pub fn save_crash(
    crash_dir: &Path,
    program: &fuzzilua_ir::Program,
    signal: i32,
    report: &str,
) -> Result<(), CorpusError> {
    fs::create_dir_all(crash_dir)?;

    let lua_source = fuzzilua_ir::lift(program);
    let hash = fnv1a_64(lua_source.as_bytes());
    let prefix = format!("crash_{hash:016x}");

    let lua_path = crash_dir.join(format!("{prefix}.lua"));
    let bin_path = crash_dir.join(format!("{prefix}.bin"));
    let txt_path = crash_dir.join(format!("{prefix}.txt"));

    atomic_write(&lua_path, lua_source.as_bytes())?;
    atomic_write(&bin_path, &bincode::serialize(program)?)?;

    let metadata = format!(
        "signal: {signal}\ntimestamp: {}\n\n{report}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    );
    atomic_write(&txt_path, metadata.as_bytes())?;

    Ok(())
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<(), CorpusError> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let tmp = path.with_file_name(format!(".tmp_{name}"));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn list_entry_paths(dir: &Path) -> Result<Vec<std::path::PathBuf>, CorpusError> {
    let mut paths: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(ENTRY_PREFIX) && n.ends_with(ENTRY_EXT))
        })
        .collect();
    paths.sort();
    Ok(paths)
}

fn fnv1a_64(data: &[u8]) -> u64 {
    const BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x00000100000001B3;
    let mut hash = BASIS;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}
