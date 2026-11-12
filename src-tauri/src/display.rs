use serde::Serialize;

/// Physical pixel size of the monitor SILO is currently displayed on.
///
/// Artwork eligibility is decided against these numbers: an image is only
/// offered as a wallpaper when its native width/height reach the monitor's
/// physical resolution and its aspect ratio matches. Nothing here is
/// hardcoded — the values come from the window's current monitor.
#[derive(Debug, Clone, Serialize)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    #[serde(rename = "scaleFactor")]
    pub scale_factor: f64,
    pub name: Option<String>,
}

#[tauri::command]
pub fn get_display_info(window: tauri::Window) -> Result<DisplayInfo, String> {
    let monitor = window
        .current_monitor()
        .map_err(|e| format!("Could not read current monitor: {}", e))?;

    let Some(monitor) = monitor else {
        return Err("No monitor is associated with the SILO window.".to_string());
    };

    let size = monitor.size();
    if size.width == 0 || size.height == 0 {
        return Err("Current monitor reported an empty resolution.".to_string());
    }

    Ok(DisplayInfo {
        width: size.width,
        height: size.height,
        scale_factor: monitor.scale_factor(),
        name: monitor.name().cloned(),
    })
}
