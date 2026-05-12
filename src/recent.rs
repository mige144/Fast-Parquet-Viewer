use std::path::PathBuf;

const MAX_RECENT: usize = 20;
const FILE_NAME: &str = "recent_files.json";

/// Returns the path to the recent-files config file (same dir as the executable).
fn config_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(dir.join(FILE_NAME))
}

/// Load the recent-files list from disk.  Returns an empty vec on any error.
pub fn load() -> Vec<String> {
    let path = match config_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<Vec<String>>(&text).unwrap_or_default()
}

/// Add `new_path` to the front of `list`, deduplicate, trim to MAX_RECENT, then persist.
pub fn push_and_save(list: &mut Vec<String>, new_path: &str) {
    list.retain(|p| p != new_path);
    list.insert(0, new_path.to_owned());
    list.truncate(MAX_RECENT);
    save(list);
}

fn save(list: &[String]) {
    let path = match config_path() {
        Some(p) => p,
        None => return,
    };
    if let Ok(text) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(path, text);
    }
}
