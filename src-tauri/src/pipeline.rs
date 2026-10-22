//! Native tool recipes and their sequential execution.
//!
//! This module coordinates the installed Modtools. It does not compile maps,
//! convert audio or create archives itself. Plans can be inspected without
//! starting any processes, which keeps compatibility checks safe and repeatable.

use crate::files::validate_name;
use crate::tools;
use crate::types::{JobRequest, JobState};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
#[cfg(not(windows))]
use std::fs::File;
#[cfg(windows)]
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Emitter;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSpec {
    pub stage: String,
    pub executable: PathBuf,
    pub arguments: String,
    pub working_directory: PathBuf,
    pub detached: bool,
    /// Products are replaced transactionally so failed tools cannot publish stale files.
    pub products: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    Process(ProcessSpec),
    Cleanup {
        source: PathBuf,
        destination: PathBuf,
    },
    Copy {
        source: PathBuf,
        destination: PathBuf,
    },
    Move {
        source: PathBuf,
        destination: PathBuf,
    },
    CreateZones {
        map_name: String,
    },
    Publish,
}

fn enabled(settings: &BTreeMap<String, String>, key: &str) -> bool {
    settings
        .get(key)
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn text<'a>(settings: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    settings.get(key).map(String::as_str).unwrap_or("")
}

fn number(settings: &BTreeMap<String, String>, key: &str) -> String {
    // Numeric controls normally supply this value. Validate here as well because
    // saved settings can be edited without going through the UI.
    let value = text(settings, key).parse::<f64>().unwrap_or(0.0);
    if value.is_finite() {
        return value.to_string();
    }
    "0".to_string()
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn process(root: &Path, executable: &str, stage: &str, arguments: String) -> ProcessSpec {
    let executable = root.join(executable);
    let working_directory = executable.parent().unwrap_or(root).to_path_buf();
    ProcessSpec {
        stage: stage.to_string(),
        executable,
        arguments,
        working_directory,
        detached: false,
        products: Vec::new(),
    }
}

pub fn bsp_options(settings: &BTreeMap<String, String>) -> String {
    let mut options = Vec::new();
    if enabled(settings, "bspoptions_onlyents") {
        options.push("-onlyents".to_string());
    }
    for (key, flag) in [("blocksize", "-blocksize"), ("samplescale", "-samplescale")] {
        if enabled(settings, &format!("bspoptions_{key}")) {
            options.push(format!(
                "{flag} {}",
                number(settings, &format!("bspoptions_{key}_val"))
            ));
        }
    }
    if enabled(settings, "bspoptions_debuglightmaps") {
        options.push("-debugLightmaps".to_string());
    }
    let custom = text(settings, "bspoptions_extraoptions").trim();
    if !custom.is_empty() {
        // The original forgot this separator and could join two unrelated flags.
        options.push(custom.to_string());
    }
    options.join(" ")
}

pub fn light_options(settings: &BTreeMap<String, String>) -> String {
    let mut options = Vec::new();
    if enabled(settings, "lightoptions_extra") {
        options.push("-extra".to_string());
    } else {
        options.push("-fast".to_string());
    }
    if enabled(settings, "lightoptions_nomodelshadow") {
        options.push("-nomodelshadow".to_string());
    } else {
        options.push("-modelshadow".to_string());
    }
    for key in ["traces", "maxbounces", "jitter"] {
        if enabled(settings, &format!("lightoptions_{key}")) {
            options.push(format!(
                "-{key} {}",
                number(settings, &format!("lightoptions_{key}_val"))
            ));
        }
    }
    if enabled(settings, "lightoptions_verbose") {
        options.push("-verbose".to_string());
    }
    options.join(" ")
}

pub fn run_arguments(settings: &BTreeMap<String, String>) -> Result<String, String> {
    let mut arguments = String::from("+set fs_game raw ");
    if enabled(settings, "run_modenabled") {
        let mod_name = text(settings, "run_modname");
        validate_name(mod_name)?;
        arguments = format!("+set fs_game mods/{mod_name} ");
    }
    for dvar in [
        "devmap",
        "thereisacow",
        "developer",
        "developer_script",
        "logfile",
        "com_introplayed",
        "sv_pure",
        "r_fullscreen",
        "sv_punkbuster",
        "scr_testclients",
    ] {
        let value = text(settings, &format!("run_{dvar}")).trim();
        if value.is_empty() {
            continue;
        }
        // devmap is a command; the other controls assign console variables.
        if dvar == "devmap" {
            arguments.push_str(&format!("+devmap {value} "));
        } else {
            arguments.push_str(&format!("+set {dvar} {value} "));
        }
    }
    arguments.push_str(text(settings, "run_custom"));
    Ok(arguments)
}

/// Describe the complete operation without starting tools or changing files.
pub fn build_plan(root: &Path, request: &JobRequest) -> Result<Vec<Step>, String> {
    let settings = &request.settings;
    let mut steps = Vec::new();
    match request.action.as_str() {
        "compile" => {
            validate_name(&request.map_name)?;
            let name = &request.map_name;
            let multiplayer = name.to_ascii_lowercase().starts_with("mp_");
            let source = root.join("map_source").join(name);
            let mut destination = root.join("raw/maps");
            if multiplayer {
                destination.push("mp");
            }
            destination.push(name);
            let game_tool = if multiplayer {
                "mp_tool.exe"
            } else {
                "sp_tool.exe"
            };
            let game = if multiplayer {
                "CoDWaWmp.exe"
            } else {
                "CoDWaW.exe"
            };
            let source_map = source.with_extension("map");
            if !source_map.is_file() {
                return Err(format!("Map source not found: {}", source_map.display()));
            }
            if enabled(settings, "compile_bsp") {
                steps.push(Step::Process(process(
                    root,
                    "bin/cod2map.exe",
                    "Compile BSP",
                    format!(
                        "-platform pc -loadFrom {} {} {}",
                        quoted(&source_map),
                        bsp_options(settings),
                        quoted(&destination)
                    ),
                )));
            }
            if enabled(settings, "compile_vis") {
                steps.push(Step::Process(process(
                    root,
                    "bin/cod2map.exe",
                    "Compile visibility",
                    format!("-vis -platform pc {}", quoted(&destination)),
                )));
            }
            if enabled(settings, "compile_lights") {
                steps.push(Step::Process(process(
                    root,
                    "bin/cod2rad.exe",
                    "Compile lighting",
                    format!(
                        "-platform pc {} {}",
                        light_options(settings),
                        quoted(&destination)
                    ),
                )));
            }
            steps.push(Step::Cleanup {
                source: source.clone(),
                destination: destination.clone(),
            });
            if enabled(settings, "compile_paths") {
                steps.push(Step::Process(process(root, game_tool, "Connect paths", format!("allowdupe +set developer 1 +set logfile 2 +set thereisacow 1337 +set com_introplayed 1 +set r_fullscreen 0 +set fs_usedevdir 1 +set g_connectpaths 2 +set useFastFile 0 +devmap {name}"))));
            }
            if enabled(settings, "compile_reflections") {
                steps.push(Step::Process(process(root, game_tool, "Build reflections", format!("allowdupe +set developer 1 +set logfile 2 +set thereisacow 1337 +set com_introplayed 1 +set r_fullscreen 0 +set fs_usedevdir 1 +set ui_autoContinue 1 +set r_reflectionProbeGenerateExit 1 +set sys_smp_allowed 0 +set useFastFile 0 +set r_fullscreen 0 +set com_hunkMegs 512 +set r_reflectionProbeRegenerateAll 1 +set r_zFeather 1 +set r_smp_backend_allowed 1 +set r_reflectionProbeGenerate 1 +devmap {name}"))));
            }
            if enabled(settings, "compile_bspinfo") {
                let info_path = destination.to_string_lossy().replace('\\', "/");
                steps.push(Step::Process(process(
                    root,
                    "bin/cod2map.exe",
                    "BSP information",
                    format!("-info \"{info_path}\""),
                )));
            }
            let mut selected_mod = None;
            if enabled(settings, "compile_modenabled") {
                let name = text(settings, "compile_modname");
                validate_name(name)?;
                selected_mod = Some(name);
            }
            let csv = root.join("zone_source").join(format!("{name}.csv"));
            let create_zone = enabled(settings, "compile_createzone");
            // The original checks zone sources at this point even when FF
            // building is disabled. Honour the user's explicit creation choice.
            if !csv.is_file() && create_zone {
                steps.push(Step::CreateZones {
                    map_name: name.to_string(),
                });
            }
            if enabled(settings, "compile_buildffs") {
                if !csv.is_file() && !create_zone {
                    return Err(format!(
                        "Missing zone source: {}. Create the zone files first.",
                        csv.display()
                    ));
                }
                let mod_option = selected_mod
                    .map(|name| format!("-moddir {name} "))
                    .unwrap_or_default();
                let load_name = format!("{name}_load");
                let has_load = root
                    .join("zone_source")
                    .join(format!("{load_name}.csv"))
                    .is_file();
                let mut zones = vec![name.to_string()];
                let load_argument = if has_load {
                    zones.push(load_name.clone());
                    format!(" {load_name}")
                } else {
                    String::new()
                };
                let mut link = process(
                    root,
                    "bin/linker_pc.exe",
                    "Build map FastFile",
                    format!("-nopause -language english {mod_option}{name}{load_argument}"),
                );
                link.products = zones
                    .iter()
                    .map(|zone| root.join("zone/english").join(format!("{zone}.ff")))
                    .collect();
                steps.push(Step::Process(link));
                if multiplayer {
                    let localized = format!("localized_{name}");
                    let mut link = process(
                        root,
                        "bin/linker_pc.exe",
                        "Build localized FastFile",
                        format!("-nopause -language english {mod_option}{localized}"),
                    );
                    link.products
                        .push(root.join("zone/english").join(format!("{localized}.ff")));
                    steps.push(Step::Process(link));
                    zones.push(localized);
                }
                let output = match selected_mod {
                    Some(name) => root.join("mods").join(name),
                    None => root.join("usermaps").join(name),
                };
                for zone in zones {
                    let filename = format!("{zone}.ff");
                    steps.push(Step::Move {
                        source: root.join("zone/english").join(&filename),
                        destination: output.join(&filename),
                    });
                }
                steps.push(Step::Publish);
            }
            if enabled(settings, "compile_runafter") {
                let fs_game = selected_mod
                    .map(|name| format!("mods/{name}"))
                    .unwrap_or_else(|| "raw".to_string());
                let arguments = if enabled(settings, "compile_useruntab") {
                    run_arguments(settings)?
                } else {
                    let mp_options = if multiplayer {
                        "+set sv_pure 0 +set g_gametype tdm "
                    } else {
                        ""
                    };
                    let custom = text(settings, "compile_runoptions");
                    format!("+set useFastFile 1 +set fs_usedevdir 1 +set logfile 2 +set thereisacow 1337 +set com_introplayed 1 {mp_options}+devmap {name} +set fs_game {fs_game} {custom}")
                };
                steps.push(Step::Process(process(root, game, "Run map", arguments)));
            }
        }
        "build_mod" => {
            validate_name(&request.mod_name)?;
            let name = &request.mod_name;
            let directory = root.join("mods").join(name);
            if !directory.is_dir() {
                return Err(format!("Mod folder not found: {}", directory.display()));
            }
            if enabled(settings, "mod_buildsounds") {
                let arguments = if enabled(settings, "mod_verbose") {
                    "-pc -ignore_orphans -verbose"
                } else {
                    "-pc -ignore_orphans"
                };
                steps.push(Step::Process(process(
                    root,
                    "bin/MODSound.exe",
                    "Build sounds",
                    arguments.to_string(),
                )));
            }
            if enabled(settings, "mod_buildff") {
                steps.push(Step::Copy {
                    source: directory.join("mod.csv"),
                    destination: root.join("zone_source/mod.csv"),
                });
                let mut link = process(
                    root,
                    "bin/linker_pc.exe",
                    "Build mod FastFile",
                    format!("-nopause -language english -moddir {name} mod"),
                );
                link.products.push(root.join("zone/english/mod.ff"));
                steps.push(Step::Process(link));
                steps.push(Step::Move {
                    source: root.join("zone/english/mod.ff"),
                    destination: directory.join("mod.ff"),
                });
            }
            if enabled(settings, "mod_buildiwd") {
                let archive = directory.join(format!("{name}.iwd"));
                let mut pack = process(
                    root,
                    "bin/7za.exe",
                    "Build IWD",
                    format!(
                        "a {} -tzip -r \"@{}\"",
                        quoted(&archive),
                        directory.join(format!("{name}.files")).display()
                    ),
                );
                pack.working_directory = directory;
                pack.products.push(archive);
                steps.push(Step::Process(pack));
            }
            steps.push(Step::Publish);
        }
        "run" => {
            let executable = if enabled(settings, "run_mp") {
                "CoDWaWmp.exe"
            } else {
                "CoDWaW.exe"
            };
            let mut run = process(root, executable, "Run game", run_arguments(settings)?);
            run.detached = true;
            steps.push(Step::Process(run));
        }
        "tool" => {
            let application = tools::find(&request.tool_id, request.companions)
                .ok_or("Unknown application shortcut.")?;
            let mut arguments = String::new();
            if request.tool_id == "radiant" && !request.map_name.is_empty() {
                validate_name(&request.map_name)?;
                let map_path = root
                    .join("map_source")
                    .join(format!("{}.map", request.map_name));
                // Stock Radiant receives the same raw path as the original
                // Launcher; its native command-line handling is not argv-based.
                arguments = if request.companions {
                    quoted(&map_path)
                } else {
                    map_path.to_string_lossy().into_owned()
                };
            }
            if request.tool_id == "converter" {
                arguments = "-nopause".to_string();
            }
            let mut launch = process(root, application.executable, application.label, arguments);
            launch.detached = request.tool_id != "converter";
            steps.push(Step::Process(launch));
        }
        "grid" => {
            validate_name(&request.map_name)?;
            let name = &request.map_name;
            let multiplayer = name.to_ascii_lowercase().starts_with("mp_");
            let directory = if multiplayer {
                "raw/maps/mp"
            } else {
                "raw/maps"
            };
            let grid = root.join(directory).join(format!("{name}.grid"));
            let source = root.join("map_source").join(format!("{name}.grid"));
            if source.is_file() {
                steps.push(Step::Copy {
                    source: source.clone(),
                    destination: grid.clone(),
                });
            }
            let mode = if enabled(settings, "grid_edit") { 2 } else { 1 };
            let cull = if enabled(settings, "compile_collectdots") {
                0
            } else {
                1
            };
            let executable = if multiplayer {
                "CoDWaWmp.exe"
            } else {
                "CoDWaW.exe"
            };
            // Preserve the original grid command's unusual spacing; no native
            // failure has established that this spelling changes its behaviour.
            steps.push(Step::Process(process(root, executable, "Edit grid", format!("+set developer 1 +set logfile 2 + set r_vc_makelog {mode}+set r_vc_showlog 16 +set r_cullxmodel {cull} +set thereisacow 1337 +set com_introplayed 1 +set fs_game raw +set fs_usedevdir 1 +devmap {name}"))));
            steps.push(Step::Move {
                source: grid,
                destination: source,
            });
        }
        _ => {
            return Err("Unknown launcher action.".to_string());
        }
    }
    Ok(steps)
}

fn log(app: &tauri::AppHandle, text: impl Into<String>, error: bool) {
    let _ = app.emit(
        "launcher-log",
        serde_json::json!({ "text": text.into(), "error": error }),
    );
}

fn create_zones(root: &Path, name: &str) -> Result<(), String> {
    use std::io::Write;
    let directory = root.join("zone_source");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let multiplayer = name.to_ascii_lowercase().starts_with("mp_");
    let mut rows = Vec::new();
    if multiplayer {
        rows.extend([
            "ignore,code_post_gfx_mp".to_string(),
            "ignore,common_mp".to_string(),
            format!("col_map_mp,maps/mp/{name}.d3dbsp"),
            format!("rawfile,maps/mp/{name}.gsc"),
            format!("rawfile,maps/mp/{name}_fx.gsc"),
        ]);
        for sound in ["common", "generic", "voiceovers", "multiplayer"] {
            rows.push(format!("sound,{sound},{name},all_mp"));
        }
    } else {
        rows.extend([
            "ignore,code_post_gfx".to_string(),
            "ignore,common".to_string(),
            format!("col_map_sp,maps/{name}.d3dbsp"),
            format!("rawfile,maps/{name}.gsc"),
            format!("rawfile,maps/{name}_anim.gsc"),
            format!("rawfile,maps/{name}_amb.gsc"),
            format!("rawfile,maps/{name}_fx.gsc"),
        ]);
        for sound in ["common", "generic", "voiceovers", "requests"] {
            rows.push(format!("sound,{sound},{name},all_sp"));
        }
    }
    // Create-only avoids destroying a CSV added or changed after the prompt.
    let path = directory.join(format!("{name}.csv"));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            let contents = format!("{}\r\n", rows.join("\r\n"));
            file.write_all(contents.as_bytes())
                .map_err(|error| error.to_string())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error.to_string());
        }
    }
    if multiplayer {
        // The original also links localized_<map>, but its CSV creator omits
        // that zone. An empty create-only zone makes that requested stage valid.
        let path = directory.join(format!("localized_{name}.csv"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error.to_string());
            }
        }
    }
    Ok(())
}

pub fn cancel(state: &JobState) {
    state.cancel.store(true, Ordering::SeqCst);
}

pub fn start(
    app: tauri::AppHandle,
    root: PathBuf,
    request: JobRequest,
    state: Arc<JobState>,
) -> Result<(), String> {
    let plan = build_plan(&root, &request)?;
    // Fail before any build or file operation if a selected tool is missing.
    for step in &plan {
        if let Step::Process(spec) = step {
            if !spec.executable.is_file() {
                return Err(format!(
                    "Application not found: {}",
                    spec.executable.display()
                ));
            }
        }
    }
    if state
        .busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Another launcher operation is still running.".to_string());
    }
    state.cancel.store(false, Ordering::SeqCst);
    thread::spawn(move || {
        let result = execute(&app, &root, &plan, &state);
        let cancelled = state.cancel.load(Ordering::SeqCst);
        let message = match &result {
            Ok(()) => "Done.".to_string(),
            Err(_) if cancelled => "Cancelled. No further stages were run.".to_string(),
            Err(message) => message.clone(),
        };
        log(&app, &message, result.is_err() && !cancelled);
        state.busy.store(false, Ordering::SeqCst);
        let _ = app.emit("launcher-finished", serde_json::json!({ "success": result.is_ok() && !cancelled, "cancelled": cancelled, "message": message }));
    });
    Ok(())
}

fn execute(
    app: &tauri::AppHandle,
    root: &Path,
    plan: &[Step],
    state: &JobState,
) -> Result<(), String> {
    for step in plan {
        if state.cancel.load(Ordering::SeqCst) {
            return Err("Cancelled.".to_string());
        }
        match step {
            Step::Process(spec) => {
                run_process(app, spec, state)?;
            }
            Step::Cleanup {
                source,
                destination,
            } => {
                smart_copy(
                    &destination.with_extension("lin"),
                    &source.with_extension("lin"),
                )?;
                for extension in ["lin", "map", "d3dpoly", "vclog", "grid"] {
                    remove_if_exists(&destination.with_extension(extension))?;
                }
            }
            Step::Copy {
                source,
                destination,
            } => {
                if !source.is_file() {
                    return Err(format!("Required file not found: {}", source.display()));
                }
                log(
                    app,
                    format!("Copying {} to {}", source.display(), destination.display()),
                    false,
                );
                smart_copy(source, destination)?;
            }
            Step::Move {
                source,
                destination,
            } => {
                log(
                    app,
                    format!("Moving {} to {}", source.display(), destination.display()),
                    false,
                );
                move_product(source, destination)?;
            }
            Step::CreateZones { map_name } => {
                create_zones(root, map_name)?;
            }
            Step::Publish => {
                let _ = app.emit(
                    "launcher-progress",
                    serde_json::json!({ "stage": "Publish" }),
                );
                publish(root, state)?;
                log(app, "Published mods and usermaps to Local AppData.", false);
            }
        }
    }
    Ok(())
}

fn run_process(app: &tauri::AppHandle, spec: &ProcessSpec, state: &JobState) -> Result<(), String> {
    let _ = app.emit(
        "launcher-progress",
        serde_json::json!({ "stage": spec.stage }),
    );
    log(
        app,
        format!(
            "{}> {} {}",
            spec.working_directory.display(),
            spec.executable.display(),
            spec.arguments
        ),
        false,
    );
    let mut command = Command::new(&spec.executable);
    command.current_dir(&spec.working_directory);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Pass the original Windows argument string directly. No shell is involved.
        command.raw_arg(&spec.arguments);
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    {
        return Err("WaW Modtools require Windows.".to_string());
    }
    if spec.detached {
        command
            .spawn()
            .map_err(|error| format!("Could not open {}: {error}", spec.stage))?;
        return Ok(());
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let backups = prepare_products(&spec.products)?;
    let result = (|| {
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start {}: {error}", spec.stage))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Missing tool output pipe.".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Missing tool error pipe.".to_string())?;
        let out_app = app.clone();
        let err_app = app.clone();
        let check_linker_errors = spec
            .executable
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("linker_pc.exe"))
            .unwrap_or(false);
        let output =
            thread::spawn(move || stream_output(out_app, stdout, false, check_linker_errors));
        let errors =
            thread::spawn(move || stream_output(err_app, stderr, true, check_linker_errors));
        let status = loop {
            if state.cancel.load(Ordering::SeqCst) {
                // Only terminate the child handle owned by this job, never all
                // game/editor processes with the same executable name.
                let _ = child.kill();
                let _ = child.wait();
                break Err("Cancelled.".to_string());
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    break Ok(status);
                }
                Ok(None) => {
                    thread::sleep(Duration::from_millis(80));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!("Could not monitor {}: {error}", spec.stage));
                }
            }
        };
        // Drain both streams before announcing completion so the last tool lines
        // cannot arrive after the following stage has started.
        let output_errors = output.join().unwrap_or(true);
        let stderr_errors = errors.join().unwrap_or(true);
        let status = status?;
        if !status.success() {
            return Err(format!(
                "{} failed (exit {}).",
                spec.stage,
                status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ));
        }
        // linker_pc can report missing assets or zone files and still exit 0.
        // Its explicit diagnostics therefore also determine stage success.
        if output_errors || stderr_errors {
            return Err(format!(
                "{} reported an error. See the tool log above.",
                spec.stage
            ));
        }
        for product in &spec.products {
            if !product.is_file() {
                return Err(format!(
                    "{} did not create {}.",
                    spec.stage,
                    product.display()
                ));
            }
        }
        Ok(())
    })();
    finish_products(backups, result.is_ok())?;
    result
}

pub fn is_linker_error(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("ERROR:")
        || line.starts_with("FATAL ERROR:")
        || line.starts_with("UNRECOVERABLE ERROR:")
}

fn stream_output<R: std::io::Read>(
    app: tauri::AppHandle,
    reader: R,
    error: bool,
    check_linker_errors: bool,
) -> bool {
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::new();
    let mut found_error = false;
    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) => {
                break;
            }
            Ok(_) => {
                let line = String::from_utf8_lossy(&bytes)
                    .trim_end_matches(['\r', '\n'])
                    .to_string();
                let diagnostic = check_linker_errors && is_linker_error(&line);
                found_error = found_error || diagnostic;
                log(&app, line, error || diagnostic);
            }
            Err(reason) => {
                log(&app, format!("Could not read tool output: {reason}"), true);
                break;
            }
        }
    }
    found_error
}

fn prepare_products(products: &[PathBuf]) -> Result<Vec<(PathBuf, Option<PathBuf>)>, String> {
    let mut backups = Vec::new();
    for product in products {
        if let Some(parent) = product.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                let rollback = finish_products(backups, false);
                return Err(format!(
                    "Could not prepare {}: {error}. {}",
                    parent.display(),
                    rollback.err().unwrap_or_default()
                ));
            }
        }
        let backup = if product.exists() {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let backup = product.with_file_name(format!(
                "{}.launcher-{timestamp}.bak",
                product.file_name().unwrap_or_default().to_string_lossy()
            ));
            if let Err(error) = fs::rename(product, &backup) {
                let _ = finish_products(backups, false);
                return Err(format!("Could not preserve {}: {error}", product.display()));
            }
            Some(backup)
        } else {
            None
        };
        backups.push((product.clone(), backup));
    }
    Ok(backups)
}

fn finish_products(backups: Vec<(PathBuf, Option<PathBuf>)>, success: bool) -> Result<(), String> {
    let mut errors = Vec::new();
    for (product, backup) in backups {
        if !success {
            if let Err(error) = remove_if_exists(&product) {
                errors.push(error);
                continue;
            }
        }
        if let Some(backup) = backup {
            if success {
                if let Err(error) = remove_if_exists(&backup) {
                    errors.push(error);
                }
            } else {
                if let Err(error) = fs::rename(&backup, &product) {
                    errors.push(format!(
                        "Could not restore {} from {}: {error}",
                        product.display(),
                        backup.display()
                    ));
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.permissions().readonly() {
            // Stock Modtools sometimes ships read-only output/support files.
            // Match the original helper, which clears that attribute first.
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions).map_err(|error| error.to_string())?;
        }
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not remove {}: {error}", path.display())),
    }
}

fn move_product(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_file() {
        return Err(format!("Expected output not found: {}", source.display()));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    remove_if_exists(destination)?;
    fs::rename(source, destination)
        .map_err(|error| format!("Could not move {}: {error}", source.display()))
}

/// Preserve the original timestamp/length comparison, including clearing a stale
/// leak marker when its source no longer exists.
pub fn smart_copy(source: &Path, destination: &Path) -> Result<(), String> {
    let source_metadata = match fs::metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return remove_if_exists(destination);
        }
        Err(error) => {
            return Err(error.to_string());
        }
    };
    if let Ok(existing) = fs::metadata(destination) {
        if source_metadata.len() == existing.len()
            && source_metadata.created().ok() == existing.created().ok()
            && source_metadata.modified().ok() == existing.modified().ok()
        {
            return Ok(());
        }
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    remove_if_exists(destination)?;
    fs::copy(source, destination)
        .map_err(|error| format!("Could not copy {}: {error}", source.display()))?;
    preserve_times(source, destination)?;
    Ok(())
}

#[cfg(windows)]
fn preserve_times(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileTime(
            handle: *mut std::ffi::c_void,
            created: *const FileTime,
            accessed: *const FileTime,
            modified: *const FileTime,
        ) -> i32;
    }
    fn file_time(value: u64) -> FileTime {
        FileTime {
            low: value as u32,
            high: (value >> 32) as u32,
        }
    }
    let metadata = fs::metadata(source).map_err(|error| error.to_string())?;
    let destination = OpenOptions::new()
        .access_mode(0x100)
        .open(destination)
        .map_err(|error| error.to_string())?;
    let created = file_time(metadata.creation_time());
    let modified = file_time(metadata.last_write_time());
    let result = unsafe {
        SetFileTime(
            destination.as_raw_handle(),
            &created,
            std::ptr::null(),
            &modified,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
fn preserve_times(source: &Path, destination: &Path) -> Result<(), String> {
    let modified = fs::metadata(source)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| error.to_string())?;
    File::options()
        .write(true)
        .open(destination)
        .and_then(|file| file.set_modified(modified))
        .map_err(|error| error.to_string())
}

fn publish(root: &Path, state: &JobState) -> Result<(), String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| "Windows Local AppData is unavailable.".to_string())?;
    let destination = PathBuf::from(local).join("Activision/CoDWaW");
    for (folder, extensions) in [
        ("usermaps", &["ff"][..]),
        ("mods", &["ff", "iwd", "arena"][..]),
    ] {
        publish_directory(
            &root.join(folder),
            &destination.join(folder),
            extensions,
            state,
        )?;
    }
    Ok(())
}

fn publish_directory(
    source: &Path,
    destination: &Path,
    extensions: &[&str],
    state: &JobState,
) -> Result<(), String> {
    if !source.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        if state.cancel.load(Ordering::SeqCst) {
            return Err("Cancelled.".to_string());
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        // Do not follow links/junction-like entries out of the selected game tree.
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            publish_directory(
                &path,
                &destination.join(entry.file_name()),
                extensions,
                state,
            )?;
        } else {
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if extensions
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            {
                smart_copy(&path, &destination.join(entry.file_name()))?;
            }
        }
    }
    Ok(())
}
