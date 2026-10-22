pub struct Application {
    pub id: &'static str,
    pub label: &'static str,
    pub executable: &'static str,
}

pub const STOCK_APPS: &[Application] = &[
    Application {
        id: "radiant",
        label: "Radiant",
        executable: "bin/CoDWaWRadiant.exe",
    },
    Application {
        id: "effects",
        label: "Effects Editor",
        executable: "bin/EffectsEd3.exe",
    },
    Application {
        id: "asset_manager",
        label: "Asset Manager",
        executable: "bin/asset_manager.exe",
    },
    Application {
        id: "asset_viewer",
        label: "Asset Viewer",
        executable: "bin/AssetViewer.exe",
    },
];

pub const COMPANION_APPS: &[Application] = &[
    Application {
        id: "asset_manager",
        label: "WaW Asset Manager",
        executable: "waw-asset-manager.exe",
    },
    Application {
        id: "ff_extractor",
        label: "WaW FF Extractor",
        executable: "waw-ff-extractor.exe",
    },
    Application {
        id: "effects",
        label: "WaW FX Viewer",
        executable: "waw-fx-viewer.exe",
    },
    Application {
        id: "hud_editor",
        label: "WaW HUD Editor",
        executable: "waw-hud-editor.exe",
    },
    Application {
        id: "image_viewer",
        label: "WaW Image Viewer",
        executable: "waw-image-viewer.exe",
    },
    Application {
        id: "asset_viewer",
        label: "WaW Model Viewer",
        executable: "waw-model-viewer.exe",
    },
    Application {
        id: "radiant",
        label: "WaW Radiant",
        executable: "waw-radiant.exe",
    },
    Application {
        id: "script_placer",
        label: "WaW Script Placer",
        executable: "waw-script-placer.exe",
    },
    Application {
        id: "shader_editor",
        label: "WaW Shader Editor",
        executable: "waw-shader-editor.exe",
    },
    Application {
        id: "visual_scripter",
        label: "WaW Visual Scripter",
        executable: "waw-visual-scripter.exe",
    },
];

pub const CONVERTER: Application = Application {
    id: "converter",
    label: "Converter",
    executable: "bin/converter.exe",
};

pub fn find(id: &str, companions: bool) -> Option<&'static Application> {
    // Converter is always the original setup tool, independent of app mode.
    if id == CONVERTER.id {
        return Some(&CONVERTER);
    }
    let applications = if companions {
        COMPANION_APPS
    } else {
        STOCK_APPS
    };
    applications.iter().find(|application| application.id == id)
}
