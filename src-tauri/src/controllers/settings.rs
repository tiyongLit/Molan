use tauri::Window;

#[tauri::command(rename_all = "snake_case")]
pub async fn set_window_size(window: Window, width: f64, height: f64) -> Result<(), String> {
    window
        .set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }))
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct WindowSize {
    width: f64,
    height: f64,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_window_size(window: Window) -> Result<WindowSize, String> {
    let physical_size = window.inner_size().map_err(|e| e.to_string())?;

    let scale_factor = window.scale_factor().map_err(|e| e.to_string())?;

    let logical_width = physical_size.width as f64 / scale_factor;
    let logical_height = physical_size.height as f64 / scale_factor;

    Ok(WindowSize {
        width: logical_width,
        height: logical_height,
    })
}
