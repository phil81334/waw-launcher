use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("Use a name containing only letters, numbers, underscores or hyphens (up to 64 characters).".into());
    }
    let reserved = name.to_ascii_uppercase();
    if [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&reserved.as_str())
    {
        return Err("That name is reserved by Windows. Choose another name.".into());
    }
    Ok(())
}

/// Keep every selected/generated relative path inside its actual directory,
/// including when an existing subdirectory is a junction or symbolic link.
pub fn safe_relative(base: &Path, relative: &str) -> Result<PathBuf, String> {
    let normalized = relative.replace('\\', "/");
    if normalized.is_empty() || normalized.contains(':') || normalized.contains('\0') {
        return Err("Invalid relative file path.".into());
    }
    let relative_path = Path::new(&normalized);
    for part in relative_path.components() {
        if !matches!(part, Component::Normal(_)) {
            return Err(format!("Path must stay inside its folder: {relative}"));
        }
    }
    let base_path = base.canonicalize().map_err(|error| error.to_string())?;
    let target = base.join(relative_path);
    let mut ancestor = target.as_path();
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or("Cannot resolve the destination folder.")?;
    }
    let actual = ancestor.canonicalize().map_err(|error| error.to_string())?;
    if !actual.starts_with(&base_path) {
        return Err(format!("Path points outside its folder: {relative}"));
    }
    Ok(target)
}

pub fn directories(path: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    names.sort_by_key(|value| value.to_ascii_lowercase());
    names
}

pub fn names_with_extension(path: &Path, extension: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let file = entry.path();
            if file.is_file()
                && file
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case(extension))
                    .unwrap_or(false)
            {
                if let Some(name) = file.file_stem().and_then(|value| value.to_str()) {
                    names.push(name.to_owned());
                }
            }
        }
    }
    names.sort_by_key(|value| value.to_ascii_lowercase());
    names
}

fn write_text(path: &Path, text: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid destination.")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let name = path
        .file_name()
        .ok_or("Invalid filename.")?
        .to_string_lossy();
    let temporary = parent.join(format!(".{name}.launcher-{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(text.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    if result.is_err() && temporary.is_file() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn load_settings(root: &Path, map: &str) -> Result<BTreeMap<String, String>, String> {
    validate_name(map)?;
    let relative = format!("bin/Launcher/map_settings/{map}.cfg");
    let path = safe_relative(root, &relative)?;
    let mut settings = BTreeMap::new();
    if !path.exists() {
        return Ok(settings);
    }
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    for line in text.trim_start_matches('\u{feff}').lines() {
        if let Some((key, value)) = line.split_once(',') {
            // Split only once: custom command text may legitimately contain commas.
            settings.insert(key.to_owned(), value.to_owned());
        }
    }
    Ok(settings)
}

pub fn save_settings(
    root: &Path,
    map: &str,
    settings: &BTreeMap<String, String>,
) -> Result<(), String> {
    validate_name(map)?;
    let path = safe_relative(root, &format!("bin/Launcher/map_settings/{map}.cfg"))?;
    let mut text = String::new();
    for (key, value) in settings {
        if key.contains([',', '\r', '\n']) || value.contains(['\r', '\n']) {
            return Err("A saved setting cannot contain a new line.".into());
        }
        text.push_str(&format!("{key},{value}\r\n"));
    }
    write_text(&path, &text)
}

#[derive(Serialize)]
pub struct ModFile {
    pub path: String,
    pub size: u64,
    pub checked: bool,
}

#[derive(Serialize)]
pub struct ModFiles {
    pub files: Vec<ModFile>,
    pub csv: String,
}

fn collect_files(base: &Path, current: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        // Do not traverse links/junctions into unrelated files or recursive loops.
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(base)
            .map_err(|error| error.to_string())?
            .to_string_lossy();
        safe_relative(base, &relative)?;
        if kind.is_dir() {
            collect_files(base, &path, output)?;
        } else if kind.is_file() {
            output.push(path);
        }
    }
    Ok(())
}

pub fn get_mod_files(root: &Path, mod_name: &str) -> Result<ModFiles, String> {
    validate_name(mod_name)?;
    let base = safe_relative(root, &format!("mods/{mod_name}"))?;
    if !base.is_dir() {
        return Err("Choose an existing mod folder.".into());
    }
    let list = base.join(format!("{mod_name}.files"));
    let checked: BTreeSet<String> = fs::read_to_string(list)
        .unwrap_or_default()
        .lines()
        .map(|line| line.trim().replace('\\', "/").to_ascii_lowercase())
        .collect();
    let mut paths = Vec::new();
    collect_files(&base, &base, &mut paths)?;
    let mut files = Vec::new();
    for path in paths {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ["ff", "iwd", "files"].contains(&extension.as_str()) {
            continue;
        }
        let relative = path
            .strip_prefix(&base)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        files.push(ModFile {
            checked: checked.contains(&relative.to_ascii_lowercase()),
            size: path.metadata().map_err(|error| error.to_string())?.len(),
            path: relative,
        });
    }
    files.sort_by_key(|file| file.path.to_ascii_lowercase());
    let csv = fs::read_to_string(base.join("mod.csv")).unwrap_or_default();
    Ok(ModFiles { files, csv })
}

pub fn save_mod_files(
    root: &Path,
    mod_name: &str,
    files: &[String],
    csv: &str,
) -> Result<(), String> {
    validate_name(mod_name)?;
    let base = safe_relative(root, &format!("mods/{mod_name}"))?;
    let mut selected = BTreeSet::new();
    for relative in files {
        if relative.contains(['\r', '\n']) {
            return Err("Invalid file selection.".into());
        }
        let path = safe_relative(&base, relative)?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !path.is_file() || ["ff", "iwd", "files"].contains(&extension.as_str()) {
            return Err(format!("Not an eligible IWD input: {relative}"));
        }
        selected.insert(relative.replace('/', "\\"));
    }
    let mut list = String::new();
    for path in selected {
        list.push_str(&format!("{path}\r\n"));
    }
    write_text(&base.join(format!("{mod_name}.files")), &list)?;
    write_text(&base.join("mod.csv"), csv)
}

#[derive(Serialize)]
pub struct CreatedMap {
    pub conflicts: Vec<String>,
    pub created: Vec<String>,
}

pub fn create_map(
    root: &Path,
    template: &str,
    map_name: &str,
    overwrite: bool,
) -> Result<CreatedMap, String> {
    validate_name(map_name)?;
    if !directories(&root.join("bin/Launcher/map_templates"))
        .iter()
        .any(|name| name == template)
    {
        return Err("Choose an installed Launcher map template.".into());
    }
    let base = safe_relative(root, &format!("bin/Launcher/map_templates/{template}"))?;
    // The original dialog strips mp_ before either template is substituted.
    let name = if map_name.to_ascii_lowercase().starts_with("mp_") {
        &map_name[3..]
    } else {
        map_name
    };
    validate_name(name)?;
    let mut inputs = Vec::new();
    collect_files(&base, &base, &mut inputs)?;
    let mut targets = Vec::new();
    let mut conflicts = Vec::new();
    for input in inputs {
        let filename = input
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if !filename.contains("template") {
            continue;
        }
        let relative = input
            .strip_prefix(&base)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/")
            .replace("template", name);
        let destination = safe_relative(root, &relative)?;
        if destination.exists() {
            conflicts.push(relative.clone());
        }
        targets.push((input, destination, relative));
    }
    conflicts.sort();
    if !conflicts.is_empty() && !overwrite {
        return Ok(CreatedMap {
            conflicts,
            created: Vec::new(),
        });
    }
    if targets.is_empty() {
        return Err("This template contains no template files.".into());
    }
    // Read every input before writing, so a bad template is reported first.
    let mut contents = Vec::new();
    for (input, destination, relative) in targets {
        let bytes = fs::read(input).map_err(|error| error.to_string())?;
        let text = String::from_utf8_lossy(&bytes);
        let mut output = String::new();
        for line in text
            .trim_start_matches('\u{feff}')
            .lines()
            .filter(|line| !line.is_empty())
        {
            output.push_str(&line.replace("template", name));
            output.push_str("\r\n");
        }
        contents.push((destination, relative, output));
    }
    let mut created = Vec::new();
    for (destination, relative, output) in contents {
        write_text(&destination, &output)?;
        created.push(relative);
    }
    created.sort();
    Ok(CreatedMap { conflicts, created })
}

pub fn get_delete_candidates(root: &Path, map_name: &str) -> Result<Vec<String>, String> {
    validate_name(map_name)?;
    let other_maps = names_with_extension(&root.join("map_source"), "map");
    let mut found = BTreeSet::new();
    for directory in ["map_source", "raw/maps", "raw/maps/mp", "zone_source"] {
        let base = safe_relative(root, directory)?;
        if !base.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&base).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
            {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = entry
                .path()
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_owned();
            let plain = stem.strip_prefix("localized_").unwrap_or(&stem);
            let matches = plain.eq_ignore_ascii_case(map_name)
                || plain
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}_", map_name.to_ascii_lowercase()));
            if !matches {
                continue;
            }
            // A second map with a shared prefix is not part of this map.
            if other_maps.iter().any(|other| {
                other
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}_", map_name.to_ascii_lowercase()))
                    && (plain.eq_ignore_ascii_case(other)
                        || plain
                            .to_ascii_lowercase()
                            .starts_with(&format!("{}_", other.to_ascii_lowercase())))
            }) {
                continue;
            }
            if directory == "map_source"
                && entry
                    .path()
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("map"))
                    .unwrap_or(false)
                && !stem.eq_ignore_ascii_case(map_name)
            {
                continue;
            }
            let relative = format!("{directory}/{name}");
            safe_relative(root, &relative)?;
            found.insert(relative);
        }
    }
    Ok(found.into_iter().collect())
}

pub fn delete_map(root: &Path, map_name: &str, files: &[String]) -> Result<(), String> {
    let allowed: BTreeSet<String> = get_delete_candidates(root, map_name)?.into_iter().collect();
    for relative in files {
        if !allowed.contains(relative) {
            return Err(format!(
                "The deletion list changed. Review it again: {relative}"
            ));
        }
    }
    for relative in files {
        let path = safe_relative(root, relative)?;
        let mut permissions = path
            .metadata()
            .map_err(|error| error.to_string())?
            .permissions();
        if permissions.readonly() {
            // The original DeleteFile supports read-only Modtools files too.
            permissions.set_readonly(false);
            fs::set_permissions(&path, permissions).map_err(|error| error.to_string())?;
        }
        fs::remove_file(&path)
            .map_err(|error| format!("Could not delete {}: {error}", path.display()))?;
    }
    Ok(())
}
