mod analysis;
mod camera;
mod engine;
mod wcf;

use std::path::PathBuf;

use tauri::ipc::Response;
use tauri::State;

use analysis::{CausticPoint, M2Result};
use camera::CameraInfo;
use engine::{Config, Engine, LogRow, Status};

type Res<T> = Result<T, String>;

fn e<E: std::fmt::Display>(err: E) -> String {
    format!("{err:#}")
}

fn merge(dst: &mut serde_json::Value, patch: serde_json::Value) {
    match (dst, patch) {
        (serde_json::Value::Object(d), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                merge(d.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (d, p) => *d = p,
    }
}

#[tauri::command]
async fn connect(engine: State<'_, Engine>, simulate: bool) -> Res<CameraInfo> {
    engine.connect(simulate).map_err(e)
}

#[tauri::command]
async fn disconnect(engine: State<'_, Engine>) -> Res<()> {
    engine.disconnect();
    Ok(())
}

#[tauri::command]
async fn status(engine: State<'_, Engine>) -> Res<Status> {
    Ok(engine.status())
}

#[tauri::command]
async fn update_config(engine: State<'_, Engine>, patch: serde_json::Value) -> Res<Config> {
    let mut cur = serde_json::to_value(engine.status().config).map_err(e)?;
    merge(&mut cur, patch);
    let cfg: Config = serde_json::from_value(cur).map_err(e)?;
    engine.set_config(cfg).map_err(e)
}

#[tauri::command]
async fn start(engine: State<'_, Engine>) -> Res<()> {
    engine.start().map_err(e)
}

#[tauri::command]
async fn stop(engine: State<'_, Engine>) -> Res<()> {
    engine.stop();
    Ok(())
}

/// Latest frame as a binary packet (empty if nothing newer than `since`).
#[tauri::command]
async fn latest_frame(engine: State<'_, Engine>, since: Option<u64>) -> Res<Response> {
    Ok(Response::new(engine.latest_packet(since.unwrap_or(u64::MAX))))
}

#[tauri::command]
async fn capture_background(engine: State<'_, Engine>, frames: usize) -> Res<()> {
    engine.capture_background(frames).map_err(e)
}

#[tauri::command]
async fn clear_background(engine: State<'_, Engine>) -> Res<()> {
    engine.clear_background();
    Ok(())
}

#[tauri::command]
async fn open_wcf(engine: State<'_, Engine>, path: PathBuf) -> Res<String> {
    engine.open_wcf(&path).map_err(e)
}

#[tauri::command]
async fn save_wcf(engine: State<'_, Engine>, path: PathBuf) -> Res<()> {
    engine.save_wcf(&path).map_err(e)
}

#[tauri::command]
async fn export_image_csv(engine: State<'_, Engine>, path: PathBuf) -> Res<()> {
    engine.export_image_csv(&path).map_err(e)
}

#[tauri::command]
async fn export_profiles_csv(engine: State<'_, Engine>, path: PathBuf) -> Res<()> {
    engine.export_profiles_csv(&path).map_err(e)
}

#[tauri::command]
async fn metrics(engine: State<'_, Engine>) -> Res<analysis::Metrics> {
    engine.metrics().map_err(e)
}

#[tauri::command]
async fn log_start(engine: State<'_, Engine>, interval_ms: f64) -> Res<()> {
    engine.log_start(interval_ms);
    Ok(())
}

#[tauri::command]
async fn log_stop(engine: State<'_, Engine>) -> Res<()> {
    engine.log_stop();
    Ok(())
}

#[tauri::command]
async fn log_clear(engine: State<'_, Engine>) -> Res<()> {
    engine.log_clear();
    Ok(())
}

#[tauri::command]
async fn log_rows(engine: State<'_, Engine>, from: usize) -> Res<Vec<LogRow>> {
    Ok(engine.log_rows(from))
}

#[tauri::command]
async fn log_export(engine: State<'_, Engine>, path: PathBuf) -> Res<()> {
    engine.log_export(&path).map_err(e)
}

#[tauri::command]
async fn fit_m2(points: Vec<CausticPoint>, wavelength_nm: f64) -> Res<M2Result> {
    Ok(analysis::fit_m2(&points, wavelength_nm))
}

/// Write UI-generated content (PNG snapshots, M² tables) to a user-chosen path.
#[tauri::command]
async fn save_bytes(path: PathBuf, data: Vec<u8>) -> Res<()> {
    std::fs::write(&path, data).map_err(e)
}

/// Append panics to `%LOCALAPPDATA%\com.gosha.beamscope\crash.log` so a vanished window can be
/// diagnosed. (Not `%LOCALAPPDATA%\BeamScope`: that is the Velopack install folder.)
fn install_crash_log() {
    let dir = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("com.gosha.beamscope");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("crash.log");
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let msg = format!("[{:.0}] thread '{thread}' {info}
{bt}

", camera::now_ms());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
            use std::io::Write;
            let _ = f.write_all(msg.as_bytes());
        }
        default(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_crash_log();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Engine::default())
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            status,
            update_config,
            start,
            stop,
            latest_frame,
            capture_background,
            clear_background,
            open_wcf,
            save_wcf,
            export_image_csv,
            export_profiles_csv,
            metrics,
            log_start,
            log_stop,
            log_clear,
            log_rows,
            log_export,
            fit_m2,
            save_bytes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
