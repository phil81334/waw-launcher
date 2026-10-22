import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { version } from "../package.json";

const element = (id) => document.getElementById(id);
const desktop = isTauri();
const controls = [...document.querySelectorAll("[data-setting]")];
const dvars = [
  ["devmap", "Loads map"],
  ["thereisacow", "Enables cheats", 1],
  ["developer", "Debug messages", 2],
  ["developer_script", "Script debugging", 1],
  ["logfile", "Console log", 2],
  ["com_introplayed", "Skip intro", 1],
  ["sv_pure", "Shipped IWDs only", 1],
  ["r_fullscreen", "Fullscreen", 1],
  ["sv_punkbuster", "PunkBuster", 2],
  ["scr_testclients", "Test clients", 10],
];

let context;
let selectedMap = "";
let selectedMod = "";
let settings = {};
let modFiles = [];
let busy = false;
let loading = false;
let startedAt = 0;
let timer;
let optionsSnapshot = null;
let confirmation = null;
let mapSelectionSerial = 0;
let modSelectionSerial = 0;
let pendingCatalog = null;

function applyPendingCatalog() {
  if (!context || loading || busy || !pendingCatalog) {
    return;
  }

  // Update names only. Automatic discovery must not save or reload the open
  // mod's file choices/CSV, or disturb the user's selected map and options.
  context.maps = pendingCatalog.maps;
  context.mods = pendingCatalog.mods;
  pendingCatalog = null;
  populateContext();
  updateRunPreview();
}

function message(text, error = false, warning = false) {
  element("message").textContent = text;
  let tone = "success";
  if (error) {
    tone = "error";
  } else if (warning) {
    tone = "warning";
  }
  element("message").className = tone;
}

function appendLog(text, error = false, warning = false) {
  const log = element("log");
  const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 35;
  const line = document.createElement("span");
  line.textContent = text;
  if (!text.endsWith("\n")) {
    line.textContent += "\n";
  }
  if (error) {
    line.className = "error";
  } else if (warning) {
    line.className = "warning";
  }
  log.append(line);
  if (atBottom) {
    log.scrollTop = log.scrollHeight;
  }
}

function addOption(select, value, text = value) {
  const item = document.createElement("option");
  item.value = value;
  item.textContent = text;
  select.append(item);
}

function isChecked(key) {
  return String(settings[key] || "").toLowerCase() === "true";
}

function collectSettings() {
  for (const control of controls) {
    if (control.type === "checkbox") {
      settings[control.dataset.setting] = control.checked ? "True" : "False";
    } else {
      settings[control.dataset.setting] = control.value;
    }
  }
  settings.lightoptions_extra = element("light-extra").checked
    ? "True"
    : "False";
  settings.run_mp = element("run-mp").checked ? "True" : "False";
  settings.run_modenabled = element("run-mod").value ? "True" : "False";
  settings.run_modname = element("run-mod").value;
  for (const [name] of dvars) {
    settings[`run_${name}`] = element(`dvar-${name}`).value;
  }
  return { ...settings };
}

function applySettings() {
  for (const control of controls) {
    if (control.type === "checkbox") {
      control.checked = isChecked(control.dataset.setting);
    } else {
      control.value = settings[control.dataset.setting] || "";
      if (control.type === "number" && control.value === "") {
        control.value = control.min || "0";
      }
      if (control.type === "number") {
        const value = Number(control.value);
        control.value = String(
          Math.min(Number(control.max), Math.max(Number(control.min), value)),
        );
      }
    }
  }
  element("light-extra").checked = isChecked("lightoptions_extra");
  element("light-fast").checked = !element("light-extra").checked;
  element("run-mp").checked = isChecked("run_mp");
  element("run-sp").checked = !element("run-mp").checked;
  element("run-mod").value = settings.run_modname || "";
  for (const [name] of dvars) {
    element(`dvar-${name}`).value = settings[`run_${name}`] || "";
    element(`values-${name}`).value = element(`dvar-${name}`).value;
  }
  updateDependencies();
  updateRunPreview();
}

function updateDependencies() {
  element("compile-mod").disabled =
    busy ||
    !document.querySelector('[data-setting="compile_modenabled"]').checked;
  const runAfter = document.querySelector(
    '[data-setting="compile_runafter"]',
  ).checked;
  const useRunTab = document.querySelector(
    '[data-setting="compile_useruntab"]',
  );
  useRunTab.disabled = busy || !runAfter;
  element("compile-run-options").disabled =
    busy || !runAfter || useRunTab.checked;
  for (const prefix of [
    "bspoptions_blocksize",
    "bspoptions_samplescale",
    "lightoptions_traces",
    "lightoptions_maxbounces",
    "lightoptions_jitter",
  ]) {
    const toggle = document.querySelector(`[data-setting="${prefix}"]`);
    const number = document.querySelector(`[data-setting="${prefix}_val"]`);
    number.disabled = busy || !toggle.checked;
  }
  element("compile").disabled = busy || !selectedMap;
  element("delete-map").disabled = busy || !selectedMap;
  element("make-grid").disabled = busy || !selectedMap;
  element("edit-grid").disabled = busy || !selectedMap;
  element("build-mod").disabled = busy || !selectedMod;
  element("save-mod").disabled = busy || !selectedMod;
  element("create-map").disabled = busy || context.templates.length === 0;
}

function setBusy(value) {
  busy = value;
  for (const control of document.querySelectorAll(
    "#app button, #app input, #app select, #app textarea",
  )) {
    control.disabled = value;
  }
  // Logs and tabs remain usable while the original tools work.
  for (const id of ["copy-log", "clear-log"]) {
    element(id).disabled = false;
  }
  for (const tab of document.querySelectorAll("[data-tab]")) {
    tab.disabled = false;
  }
  element("cancel").disabled = !value;
  updateDependencies();
  if (value) {
    startedAt = Date.now();
    timer = setInterval(() => {
      const seconds = Math.floor((Date.now() - startedAt) / 1000);
      element("elapsed").textContent =
        `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
    }, 1000);
  } else {
    clearInterval(timer);
    applyPendingCatalog();
  }
}

async function saveMap() {
  if (!selectedMap || !desktop) {
    return;
  }
  await invoke("save_settings", {
    mapName: selectedMap,
    settings: collectSettings(),
  });
}

async function saveMod() {
  if (!selectedMod || !desktop) {
    return;
  }
  await invoke("save_mod_files", {
    modName: selectedMod,
    files: modFiles.filter((file) => file.checked).map((file) => file.path),
    csv: element("mod-csv").value,
  });
}

async function selectMap(name) {
  const serial = ++mapSelectionSerial;
  try {
    await saveMap();
    if (serial !== mapSelectionSerial) {
      return;
    }
    let loadedSettings = {};
    if (desktop && name) {
      loadedSettings = await invoke("load_settings", { mapName: name });
    }
    if (serial !== mapSelectionSerial) {
      return;
    }
    selectedMap = name;
    settings = loadedSettings;
    applySettings();
    element("selected-map").textContent = name || "Select a map.";
  } catch (error) {
    message(String(error), true);
  }
}

function renderMaps() {
  const list = element("maps");
  const query = element("map-search").value.toLowerCase().trim();
  list.replaceChildren();
  for (const name of context.maps) {
    if (name.toLowerCase().includes(query)) {
      addOption(list, name);
    }
  }
  list.value = selectedMap;
}

function updateFileCount() {
  const count = modFiles.filter((file) => file.checked).length;
  element("file-count").textContent = `${count} of ${modFiles.length} selected`;
}

function renderFileTree() {
  const root = { folders: new Map(), files: [] };
  for (const file of modFiles) {
    const parts = file.path.replaceAll("\\", "/").split("/");
    let node = root;
    for (const part of parts.slice(0, -1)) {
      if (!node.folders.has(part)) {
        node.folders.set(part, { folders: new Map(), files: [] });
      }
      node = node.folders.get(part);
    }
    node.files.push(file);
  }

  const directoryChecks = [];
  function addNodes(node, container) {
    const descendants = [];
    for (const [name, child] of node.folders) {
      const details = document.createElement("details");
      details.open = true;
      const summary = document.createElement("summary");
      const label = document.createElement("label");
      const check = document.createElement("input");
      check.type = "checkbox";
      label.append(check, document.createTextNode(name));
      summary.append(label);
      details.append(summary);
      container.append(details);
      const children = addNodes(child, details);
      descendants.push(...children);
      directoryChecks.push([check, children]);
      check.addEventListener("click", (event) => event.stopPropagation());
      check.addEventListener("change", () => {
        for (const file of children) {
          file.checked = check.checked;
        }
        renderFileTree();
      });
    }
    for (const file of node.files) {
      const label = document.createElement("label");
      label.className = "file-row";
      const check = document.createElement("input");
      check.type = "checkbox";
      check.checked = file.checked;
      check.disabled = busy;
      check.addEventListener("change", () => {
        file.checked = check.checked;
        updateDirectories();
        updateFileCount();
      });
      label.append(
        check,
        document.createTextNode(file.path.split(/[\\/]/).at(-1)),
      );
      label.title = file.path;
      container.append(label);
      descendants.push(file);
    }
    return descendants;
  }

  function updateDirectories() {
    for (const [check, files] of directoryChecks) {
      const checked = files.filter((file) => file.checked).length;
      check.checked = checked > 0 && checked === files.length;
      check.indeterminate = checked > 0 && checked < files.length;
      check.disabled = busy;
    }
  }

  const tree = element("file-tree");
  tree.replaceChildren();
  addNodes(root, tree);
  if (modFiles.length === 0) {
    const empty = document.createElement("p");
    empty.className = "hint";
    empty.textContent = selectedMod
      ? "No files available for IWD packing."
      : "Select a mod.";
    tree.append(empty);
  }
  updateDirectories();
  updateFileCount();
}

async function selectMod(name) {
  const serial = ++modSelectionSerial;
  try {
    await saveMod();
    if (serial !== modSelectionSerial) {
      return;
    }
    const result =
      desktop && name
        ? await invoke("get_mod_files", { modName: name })
        : { files: [], csv: "" };
    if (serial !== modSelectionSerial) {
      return;
    }
    selectedMod = name;
    modFiles = result.files;
    element("mod-csv").value = result.csv;
    renderFileTree();
    updateDependencies();
  } catch (error) {
    message(String(error), true);
  }
}

function renderTools() {
  const container = element("tools");
  container.replaceChildren();
  const companions = element("companions").checked;
  for (const tool of context.tools) {
    if (tool.companion !== companions) {
      continue;
    }
    const button = document.createElement("button");
    const label = document.createElement("span");
    label.textContent = tool.label;
    const filename = document.createElement("small");
    filename.textContent = tool.executable;
    button.append(label, filename);
    button.title = tool.executable;
    button.disabled = busy;
    button.addEventListener("click", () =>
      startJob("tool", { toolId: tool.id }),
    );
    container.append(button);
  }
  element("tool-mode").textContent = companions
    ? "Companion apps run from beside CoDWaW.exe. An app that is not installed shows a warning in the log. Build tools always use the original Modtools."
    : "Original Modtools applications in bin. Build tools always use the installed Modtools.";
}

function createDvars() {
  const container = element("dvars");
  for (const [name, description, maximum] of dvars) {
    const label = document.createElement("label");
    label.htmlFor = `dvar-${name}`;
    label.textContent = name;
    label.title = description;
    const input = document.createElement("input");
    input.type = "text";
    input.id = `dvar-${name}`;
    input.placeholder = "(not set)";
    input.title = description;
    const list = document.createElement("select");
    list.id = `values-${name}`;
    list.setAttribute("aria-label", `Choose ${name} value`);
    addOption(list, "", "(not set)");
    if (maximum !== undefined) {
      for (let value = 0; value <= maximum; value += 1) {
        addOption(list, String(value));
      }
    }
    // Keep free typing, but use an unfiltered native picker so (not set)
    // stays available after choosing a value. A datalist hides other matches.
    input.addEventListener("input", () => {
      list.value = input.value;
      updateRunPreview();
    });
    list.addEventListener("change", () => {
      input.value = list.value;
      updateRunPreview();
    });

    const clear = document.createElement("button");
    clear.type = "button";
    clear.textContent = "Clear";
    clear.title = `Set ${name} to (not set)`;
    clear.setAttribute("aria-label", `Clear ${name}`);
    clear.addEventListener("click", () => {
      input.value = "";
      list.value = "";
      updateRunPreview();
    });

    const value = document.createElement("div");
    value.className = "dvar-value";
    value.append(list, input);
    const control = document.createElement("div");
    control.className = "dvar-control";
    control.append(value, clear);
    container.append(label, control);
  }
}

function updateRunPreview() {
  const mod = element("run-mod").value;
  const parts = ["+set fs_game", mod ? `mods/${mod}` : "raw"];
  for (const [name] of dvars) {
    const value = element(`dvar-${name}`).value.trim();
    if (value) {
      // devmap runs a command rather than assigning a console variable.
      if (name === "devmap") {
        parts.push(`+devmap ${value}`);
      } else {
        parts.push(`+set ${name} ${value}`);
      }
    }
  }
  const custom = element("run-custom").value;
  if (custom) {
    parts.push(custom);
  }
  element("command-preview").value = parts.join(" ");
}

function confirmAction(title, text, files = []) {
  element("confirm-title").textContent = title;
  element("confirm-message").textContent = text;
  element("confirm-files").textContent = files.join("\n");
  element("confirm-files").hidden = files.length === 0;
  element("confirm-dialog").showModal();
  return new Promise((resolve) => {
    confirmation = resolve;
  });
}

function finishConfirmation(accepted) {
  element("confirm-dialog").close();
  if (confirmation) {
    confirmation(accepted);
    confirmation = null;
  }
}

async function startJob(action, extra = {}) {
  if (busy) {
    return;
  }
  if (!desktop) {
    message(
      "Browser preview only. Run npm run dev to use your installed Modtools.",
    );
    return;
  }
  try {
    const currentSettings = collectSettings();
    if (action === "compile") {
      // A previous compile may have created the zone source since the last scan.
      const latest = await invoke("refresh_catalog");
      context.zoneSources = latest.zoneSources || [];
    }
    if (action === "compile" && !context.zoneSources.includes(selectedMap)) {
      const accepted = await confirmAction(
        "Zone source missing",
        `There are no zone source files for ${selectedMap}. Create the original Launcher's default files?`,
      );
      if (!accepted) {
        return;
      }
      currentSettings.compile_createzone = "True";
    }
    await saveMap();
    if (action === "build_mod") {
      await saveMod();
    }
    setBusy(true);
    element("status").textContent = "Working…";
    element("current-stage").textContent = "Starting…";
    message("");
    const warning = await invoke("start_job", {
      request: {
        action,
        mapName: selectedMap,
        modName: selectedMod,
        settings: currentSettings,
        files: modFiles.filter((file) => file.checked).map((file) => file.path),
        companions: element("companions").checked,
        ...extra,
      },
    });
    if (warning) {
      setBusy(false);
      element("status").textContent = "Warning";
      element("current-stage").textContent = "Log";
      appendLog(warning, false, true);
      message(warning, false, true);
    }
  } catch (error) {
    setBusy(false);
    element("status").textContent = "Error";
    element("current-stage").textContent = "Log";
    appendLog(String(error), true);
    message(String(error), true);
  }
}

function openOptions(id) {
  optionsSnapshot = collectSettings();
  element(id).showModal();
}

function closeOptions(id, save) {
  if (save) {
    collectSettings();
  } else if (optionsSnapshot) {
    settings = optionsSnapshot;
    applySettings();
  }
  optionsSnapshot = null;
  element(id).close();
}

function switchTab(name) {
  for (const tab of document.querySelectorAll("[data-tab]")) {
    const active = tab.dataset.tab === name;
    tab.classList.toggle("active", active);
    tab.setAttribute("aria-selected", String(active));
    tab.tabIndex = active ? 0 : -1;
    element(`${tab.dataset.tab}-panel`).hidden = !active;
  }
}

async function refresh() {
  if (loading || busy) {
    return;
  }
  loading = true;
  try {
    await saveMap();
    await saveMod();
    context = desktop ? await invoke("refresh_catalog") : browserContext();
    populateContext();
    if (selectedMod && desktop) {
      // The edited file list is already saved before reading the refreshed tree.
      const result = await invoke("get_mod_files", { modName: selectedMod });
      modFiles = result.files;
      element("mod-csv").value = result.csv;
      renderFileTree();
    }
    message(
      `Found ${context.maps.length} maps and ${context.mods.length} mods.`,
    );
  } catch (error) {
    message(String(error), true);
  } finally {
    loading = false;
    applyPendingCatalog();
  }
}

function populateContext() {
  context.zoneSources ||= [];
  if (selectedMap && !context.maps.includes(selectedMap)) {
    mapSelectionSerial += 1;
    selectedMap = "";
    settings = {};
    applySettings();
    element("selected-map").textContent = "Select a map.";
  }
  if (selectedMod && !context.mods.includes(selectedMod)) {
    modSelectionSerial += 1;
    selectedMod = "";
    modFiles = [];
    element("mod-csv").value = "";
    renderFileTree();
  }
  element("app-version").textContent = `WaW Launcher | ${context.version}`;
  element("environment").textContent = context.development
    ? "Development"
    : "Production";
  renderMaps();
  for (const id of ["mod", "compile-mod", "run-mod"]) {
    const select = element(id);
    const previous = select.value;
    select.replaceChildren();
    addOption(select, "", id === "run-mod" ? "raw" : "Select a mod…");
    for (const name of context.mods) {
      addOption(select, name);
    }
    select.value = previous;
  }
  element("template").replaceChildren();
  for (const name of context.templates) {
    addOption(element("template"), name);
  }
  element("create-map").disabled = context.templates.length === 0;
  const mapChoices = element("values-devmap");
  mapChoices.replaceChildren();
  addOption(mapChoices, "", "(not set)");
  for (const name of context.maps) {
    addOption(mapChoices, name);
  }
  mapChoices.value = element("dvar-devmap").value;
  renderTools();
  updateDependencies();
}

async function createMap() {
  if (!desktop) {
    message(
      "Browser preview only. Map creation is available in the desktop app.",
    );
    return;
  }
  const mapName = element("new-map").value.trim();
  const template = element("template").value;
  if (!mapName || !template) {
    message("Choose a template and enter a map name.", true);
    return;
  }
  try {
    let result = await invoke("create_map", {
      template,
      mapName,
      overwrite: false,
    });
    if (result.conflicts.length > 0) {
      // Close the input dialog while the exact overwrite list is reviewed.
      element("create-dialog").close();
      const accepted = await confirmAction(
        "Replace existing map files?",
        "These files already exist. Continue only if you want to replace them.",
        result.conflicts,
      );
      if (!accepted) {
        return;
      }
      result = await invoke("create_map", {
        template,
        mapName,
        overwrite: true,
      });
    }
    element("create-dialog").close();
    await refresh();
    element("maps").value = mapName;
    await selectMap(mapName);
    message(`Created ${result.created.length} files for ${mapName}.`);
  } catch (error) {
    message(String(error), true);
  }
}

async function deleteMap() {
  if (!desktop || !selectedMap) {
    return;
  }
  try {
    const mapName = selectedMap;
    const files = await invoke("get_delete_candidates", { mapName });
    const accepted = await confirmAction(
      `Delete ${mapName}?`,
      "The following files will be permanently deleted.",
      files,
    );
    if (!accepted) {
      return;
    }
    await invoke("delete_map", { mapName, files });
    selectedMap = "";
    settings = {};
    applySettings();
    await refresh();
    message(`Deleted ${files.length} files for ${mapName}.`);
  } catch (error) {
    message(String(error), true);
  }
}

function browserContext() {
  return {
    validRoot: true,
    development: true,
    version,
    maps: ["mp_barebones", "nazi_zombie_tutorial"],
    mods: [],
    templates: [],
    zoneSources: [],
    tools: [
      ...[
        ["radiant", "Radiant", "bin/CoDWaWRadiant.exe"],
        ["effects", "Effects Editor", "bin/EffectsEd3.exe"],
        ["asset_manager", "Asset Manager", "bin/asset_manager.exe"],
        ["asset_viewer", "Asset Viewer", "bin/AssetViewer.exe"],
      ].map(([id, label, executable]) => ({ id, label, executable, companion: false })),
      ...[
        ["asset_manager", "WaW Asset Manager", "waw-asset-manager.exe"],
        ["ff_extractor", "WaW FF Extractor", "waw-ff-extractor.exe"],
        ["effects", "WaW FX Viewer", "waw-fx-viewer.exe"],
        ["hud_editor", "WaW HUD Editor", "waw-hud-editor.exe"],
        ["image_viewer", "WaW Image Viewer", "waw-image-viewer.exe"],
        ["asset_viewer", "WaW Model Viewer", "waw-model-viewer.exe"],
        ["radiant", "WaW Radiant", "waw-radiant.exe"],
        ["script_placer", "WaW Script Placer", "waw-script-placer.exe"],
        ["shader_editor", "WaW Shader Editor", "waw-shader-editor.exe"],
        ["visual_scripter", "WaW Visual Scripter", "waw-visual-scripter.exe"],
      ].map(([id, label, executable]) => ({ id, label, executable, companion: true })),
    ],
  };
}

function bindEvents() {
  for (const tab of document.querySelectorAll("[data-tab]")) {
    tab.addEventListener("click", () => switchTab(tab.dataset.tab));
    tab.addEventListener("keydown", (event) => {
      const tabs = [...document.querySelectorAll("[data-tab]")];
      let index = tabs.indexOf(tab);
      if (event.key === "ArrowRight") {
        index = (index + 1) % tabs.length;
      } else if (event.key === "ArrowLeft") {
        index = (index + tabs.length - 1) % tabs.length;
      } else {
        return;
      }
      event.preventDefault();
      switchTab(tabs[index].dataset.tab);
      tabs[index].focus();
    });
  }
  for (const control of controls) {
    control.addEventListener("change", () => {
      collectSettings();
      updateDependencies();
      updateRunPreview();
    });
  }
  for (const id of ["run-sp", "run-mp", "run-mod", "run-custom"]) {
    element(id).addEventListener("input", updateRunPreview);
  }
  element("map-search").addEventListener("input", renderMaps);
  element("maps").addEventListener("change", () =>
    selectMap(element("maps").value),
  );
  element("mod").addEventListener("change", () =>
    selectMod(element("mod").value),
  );
  element("refresh").addEventListener("click", refresh);
  element("companions").addEventListener("change", renderTools);
  element("converter").addEventListener("click", () =>
    startJob("tool", { toolId: "converter", companions: false }),
  );
  element("compile").addEventListener("click", () => startJob("compile"));
  element("build-mod").addEventListener("click", () => startJob("build_mod"));
  element("run").addEventListener("click", () => startJob("run"));
  element("make-grid").addEventListener("click", () => {
    collectSettings();
    settings.grid_edit = "False";
    startJob("grid");
  });
  element("edit-grid").addEventListener("click", () => {
    collectSettings();
    settings.grid_edit = "True";
    startJob("grid");
  });
  element("save-mod").addEventListener("click", async () => {
    try {
      await saveMod();
      message("Saved the IWD file list and mod.csv.");
    } catch (error) {
      message(String(error), true);
    }
  });
  for (const [id, checked] of [
    ["check-files", true],
    ["uncheck-files", false],
  ]) {
    element(id).addEventListener("click", () => {
      for (const file of modFiles) {
        file.checked = checked;
      }
      renderFileTree();
    });
  }
  element("bsp-options").addEventListener("click", () =>
    openOptions("bsp-dialog"),
  );
  element("light-options").addEventListener("click", () =>
    openOptions("light-dialog"),
  );
  for (const button of document.querySelectorAll("[data-options-save]")) {
    button.addEventListener("click", () =>
      closeOptions(button.dataset.optionsSave, true),
    );
  }
  for (const button of document.querySelectorAll("[data-options-cancel]")) {
    button.addEventListener("click", () =>
      closeOptions(button.dataset.optionsCancel, false),
    );
  }
  for (const id of ["bsp-dialog", "light-dialog"]) {
    element(id).addEventListener("cancel", (event) => {
      event.preventDefault();
      closeOptions(id, false);
    });
  }
  element("create-map").addEventListener("click", () =>
    element("create-dialog").showModal(),
  );
  element("confirm-create").addEventListener("click", createMap);
  element("delete-map").addEventListener("click", deleteMap);
  element("confirm-yes").addEventListener("click", () =>
    finishConfirmation(true),
  );
  element("confirm-no").addEventListener("click", () =>
    finishConfirmation(false),
  );
  element("confirm-dialog").addEventListener("cancel", (event) => {
    event.preventDefault();
    finishConfirmation(false);
  });
  element("clear-log").addEventListener("click", () =>
    element("log").replaceChildren(),
  );
  element("copy-log").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(element("log").textContent);
      message("Log copied.");
    } catch (error) {
      message(`Could not copy the log: ${error}`, true);
    }
  });
  element("cancel").addEventListener("click", async () => {
    try {
      await invoke("cancel_job");
      element("cancel").disabled = true;
      element("status").textContent = "Cancelling…";
    } catch (error) {
      message(String(error), true);
    }
  });
  element("about").addEventListener("click", () => {
    element("about-dialog").showModal();
    element("repo-url").focus();
    element("repo-url").select();
  });
  element("repo-url").addEventListener("click", (event) =>
    event.target.select(),
  );
  for (const button of document.querySelectorAll("[data-close]")) {
    button.addEventListener("click", () =>
      element(button.dataset.close).close(),
    );
  }
  element("quit").addEventListener("click", async () => {
    if (desktop) {
      await getCurrentWindow().close();
    }
  });
  // The placement message must remain visible until the user closes the app.
  element("startup-error").addEventListener("cancel", (event) => {
    event.preventDefault();
  });
}

async function initialise() {
  createDvars();
  bindEvents();
  try {
    if (desktop) {
      await listen("launcher-catalog-changed", (event) => {
        pendingCatalog = event.payload;
        applyPendingCatalog();
      });
      // Register before starting a job, including very short converter runs.
      await listen("launcher-log", (event) =>
        appendLog(event.payload.text, event.payload.error),
      );
      await listen("launcher-progress", (event) => {
        element("current-stage").textContent = event.payload.stage;
        element("status").textContent = event.payload.stage;
      });
      await listen("launcher-finished", (event) => {
        setBusy(false);
        element("current-stage").textContent = "Log";
        if (event.payload.cancelled) {
          element("status").textContent = "Cancelled";
        } else if (event.payload.success) {
          element("status").textContent = "Done";
        } else {
          element("status").textContent = "Error";
        }
        message(
          event.payload.message,
          !event.payload.success && !event.payload.cancelled,
        );
      });
      context = await invoke("get_context");
    } else {
      context = browserContext();
    }
    if (!context.validRoot) {
      element("startup-message").textContent = context.message;
      element("startup-error").showModal();
      return;
    }
    populateContext();
    applyPendingCatalog();
    applySettings();
    renderFileTree();
    element("app").hidden = false;
    element("footer").hidden = false;
    if (!desktop) {
      message(
        "Browser preview only. Use npm run dev for the desktop app and installed Modtools.",
      );
    }
    if (desktop) {
      // Save editor changes on close; never stop an unrelated game process.
      await getCurrentWindow().onCloseRequested(async (event) => {
        event.preventDefault();
        if (busy) {
          const accepted = await confirmAction(
            "Close Launcher?",
            "A tool is still running. Cancel this Launcher's current job and close?",
          );
          if (!accepted) {
            return;
          }
          await invoke("cancel_job");
          // Wait for the owned child to stop before destroying its event loop.
          const deadline = Date.now() + 10000;
          while (busy && Date.now() < deadline) {
            await new Promise((resolve) => setTimeout(resolve, 100));
          }
          if (busy) {
            message(
              "Still cancelling the current tool. Wait for it to stop before closing.",
              true,
            );
            return;
          }
        }
        try {
          await saveMap();
          await saveMod();
          await getCurrentWindow().destroy();
        } catch (error) {
          message(`Could not save changes: ${error}`, true);
        }
      });
    }
  } catch (error) {
    element("startup-message").textContent =
      `Could not start WaW Launcher: ${error}`;
    element("startup-error").showModal();
  }
}

initialise();
