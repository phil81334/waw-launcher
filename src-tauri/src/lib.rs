mod catalog;
pub mod files;
pub mod pipeline;
mod tools;
pub mod types;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Manager;
use types::{Context, JobRequest, JobState, Tool};

struct AppState {
    root: PathBuf,
    jobs: Arc<JobState>,
}

fn resolve_root() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Some(path) = std::env::var_os("WAW_ROOT") {
            return Ok(PathBuf::from(path));
        }
        // Development never defaults to the installed game. The path can be
        // changed explicitly using .local/game-path.txt or WAW_ROOT.
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.local/test-root");
        for folder in [
            "bin/Launcher/map_settings",
            "map_source",
            "mods",
            "usermaps",
            "raw/maps/mp",
            "zone_source",
            "zone/english",
        ] {
            std::fs::create_dir_all(root.join(folder)).map_err(|error| error.to_string())?;
        }
        return root.canonicalize().map_err(|error| error.to_string());
    }
    std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "Cannot locate the app folder.".into())
}

fn ensure_root(state: &AppState) -> Result<(), String> {
    if !state.root.is_dir() {
        return Err("The configured game folder does not exist.".into());
    }
    if !cfg!(debug_assertions) && !state.root.join("CoDWaW.exe").is_file() {
        return Err("Put this executable in your World at War folder, beside CoDWaW.exe, then open it again.".into());
    }
    Ok(())
}

fn ensure_idle(state: &AppState) -> Result<(), String> {
    ensure_root(state)?;
    if state.jobs.busy.load(Ordering::SeqCst) {
        return Err("Wait for the current task to finish, or cancel it first.".into());
    }
    Ok(())
}

fn context(state: &AppState) -> Context {
    let error = ensure_root(state).err();
    let tools = tools::STOCK_APPS
        .iter()
        .map(|application| (application, false))
        .chain(
            tools::COMPANION_APPS
                .iter()
                .map(|application| (application, true)),
        )
        .map(|(application, companion)| Tool {
            id: application.id.into(),
            label: application.label.into(),
            executable: application.executable.into(),
            companion,
        })
        .collect();
    let message = error.clone().unwrap_or_else(|| {
        if cfg!(debug_assertions) && !state.root.join("CoDWaW.exe").is_file() {
            "Development uses the local test folder. Install tools there or set .local/game-path.txt to an explicit Modtools copy.".into()
        } else {
            String::new()
        }
    });
    Context {
        valid_root: error.is_none(),
        root: state.root.to_string_lossy().into_owned(),
        message,
        development: cfg!(debug_assertions),
        version: env!("CARGO_PKG_VERSION").into(),
        maps: files::names_with_extension(&state.root.join("map_source"), "map"),
        mods: files::directories(&state.root.join("mods")),
        templates: files::directories(&state.root.join("bin/Launcher/map_templates")),
        zone_sources: files::names_with_extension(&state.root.join("zone_source"), "csv"),
        tools,
    }
}

#[tauri::command]
fn get_context(state: tauri::State<AppState>) -> Context {
    context(&state)
}

#[tauri::command]
fn refresh_catalog(state: tauri::State<AppState>) -> Context {
    context(&state)
}

#[tauri::command]
fn load_settings(
    state: tauri::State<AppState>,
    map_name: String,
) -> Result<BTreeMap<String, String>, String> {
    ensure_root(&state)?;
    files::load_settings(&state.root, &map_name)
}

#[tauri::command]
fn save_settings(
    state: tauri::State<AppState>,
    map_name: String,
    settings: BTreeMap<String, String>,
) -> Result<(), String> {
    ensure_idle(&state)?;
    files::save_settings(&state.root, &map_name, &settings)
}

#[tauri::command]
fn get_mod_files(
    state: tauri::State<AppState>,
    mod_name: String,
) -> Result<files::ModFiles, String> {
    ensure_root(&state)?;
    files::get_mod_files(&state.root, &mod_name)
}

#[tauri::command]
fn save_mod_files(
    state: tauri::State<AppState>,
    mod_name: String,
    files: Vec<String>,
    csv: String,
) -> Result<(), String> {
    ensure_idle(&state)?;
    files::save_mod_files(&state.root, &mod_name, &files, &csv)
}

#[tauri::command]
fn create_map(
    state: tauri::State<AppState>,
    template: String,
    map_name: String,
    overwrite: bool,
) -> Result<files::CreatedMap, String> {
    ensure_idle(&state)?;
    files::create_map(&state.root, &template, &map_name, overwrite)
}

#[tauri::command]
fn get_delete_candidates(
    state: tauri::State<AppState>,
    map_name: String,
) -> Result<Vec<String>, String> {
    ensure_idle(&state)?;
    files::get_delete_candidates(&state.root, &map_name)
}

#[tauri::command]
fn delete_map(
    state: tauri::State<AppState>,
    map_name: String,
    files: Vec<String>,
) -> Result<(), String> {
    ensure_idle(&state)?;
    files::delete_map(&state.root, &map_name, &files)
}

#[tauri::command]
fn start_job(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    request: JobRequest,
) -> Result<Option<String>, String> {
    ensure_root(&state)?;
    if request.action == "tool" && request.companions && request.tool_id != "converter" {
        let application =
            tools::find(&request.tool_id, true).ok_or("Unknown companion application.")?;
        if !state.root.join(application.executable).is_file() {
            return Ok(Some(format!(
                "Warning: {} was not found beside CoDWaW.exe. Nothing was launched.",
                application.executable
            )));
        }
    }
    pipeline::start(app, state.root.clone(), request, state.jobs.clone())?;
    Ok(None)
}

#[tauri::command]
fn cancel_job(state: tauri::State<AppState>) {
    state.jobs.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn quit(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    ensure_idle(&state)?;
    app.exit(0);
    Ok(())
}

pub fn run() {
    // Keep WebView data away from the game folder, matching the other family apps.
    let profile = tempfile::Builder::new()
        .prefix("waw-launcher-ui-")
        .tempdir()
        .expect("Cannot create the temporary browser profile.");
    let profile_path = profile.path().to_path_buf();
    let root = resolve_root().unwrap_or_default();
    tauri::Builder::default()
        .manage(AppState {
            root,
            jobs: Arc::new(JobState::default()),
        })
        .setup(move |app| {
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .data_directory(profile_path)
                .build()?;
            let state = app.state::<AppState>();
            if ensure_root(&state).is_ok() {
                if let Err(error) = catalog::start(app.handle().clone(), state.root.clone()) {
                    eprintln!("Cannot start automatic map/mod detection: {error}");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_context,
            refresh_catalog,
            load_settings,
            save_settings,
            get_mod_files,
            save_mod_files,
            create_map,
            get_delete_candidates,
            delete_map,
            start_job,
            cancel_job,
            quit
        ])
        .run(tauri::generate_context!())
        .expect("Cannot start WaW Launcher.");
}
