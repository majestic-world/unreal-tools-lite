mod app_settings;
mod geodata;
pub mod texture_engine;
mod texture_resize;
mod utx;

use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex, MutexGuard},
};

use serde::Deserialize;
use tauri::{Emitter, Manager};

struct UtxCache(Arc<Mutex<Option<utx::CachedUtx>>>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UtxTexturePropertiesBatchEdit {
    export_index: usize,
    edit: texture_engine::TextureEditorEdit,
}

fn cached_utx<'a>(
    cache: &'a UtxCache,
    file_path: &str,
) -> Result<MutexGuard<'a, Option<utx::CachedUtx>>, String> {
    let guard = cache
        .0
        .lock()
        .map_err(|_| "Não foi possível acessar o cache do pacote UTX.".to_string())?;
    match guard.as_ref() {
        Some(session) if session.matches_path(file_path) => Ok(guard),
        Some(_) => Err("O pacote UTX aberto foi alterado. Abra o arquivo novamente.".into()),
        None => Err("Nenhum pacote UTX está aberto. Abra o arquivo novamente.".into()),
    }
}

fn app_settings_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let directory = app.path().app_data_dir().map_err(|error| {
        format!("Não foi possível localizar a pasta AppData do aplicativo: {error}")
    })?;
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "Não foi possível criar a pasta de configurações em '{}': {error}",
            directory.display()
        )
    })?;
    Ok(directory)
}

fn app_logs_directory() -> Result<PathBuf, String> {
    let directory = std::env::temp_dir().join("unreal-tools");
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "Não foi possível criar a pasta de logs em '{}': {error}",
            directory.display()
        )
    })?;
    Ok(directory)
}

#[tauri::command]
fn app_settings_load(app: tauri::AppHandle) -> Result<app_settings::AppSettings, String> {
    let directory = app_settings_directory(&app)?;
    app_settings::load(&directory)
}

#[tauri::command]
fn app_settings_save(
    app: tauri::AppHandle,
    settings: app_settings::AppSettings,
) -> Result<(), String> {
    let directory = app_settings_directory(&app)?;
    app_settings::save(&directory, &settings)
}

#[tauri::command]
fn app_settings_open_directory(app: tauri::AppHandle) -> Result<String, String> {
    let directory = app_settings_directory(&app)?;
    Command::new("explorer.exe")
        .arg(&directory)
        .spawn()
        .map_err(|error| format!("Não foi possível abrir a pasta de configurações: {error}"))?;
    Ok(directory.to_string_lossy().into_owned())
}

#[tauri::command]
fn app_logs_open_directory() -> Result<String, String> {
    let directory = app_logs_directory()?;
    Command::new("explorer.exe")
        .arg(&directory)
        .spawn()
        .map_err(|error| format!("Não foi possível abrir a pasta de logs: {error}"))?;
    Ok(directory.to_string_lossy().into_owned())
}

#[tauri::command]
fn utx_list_entries(file_path: String) -> Result<Vec<utx::UtxEntry>, String> {
    utx::list_entries(&file_path)
}

#[tauri::command]
fn utx_create_new(file_path: String) -> Result<(), String> {
    utx::create_new(&file_path)
}

#[tauri::command]
fn utx_export_entry(
    file_path: String,
    export_index: usize,
    output_path: String,
) -> Result<(), String> {
    utx::export_entry(&file_path, export_index, &output_path)
}

#[tauri::command]
fn utx_export_entries(
    file_path: String,
    export_indices: Vec<usize>,
    output_dir: String,
) -> Result<utx::ExportSummary, String> {
    utx::export_entries(&file_path, export_indices, &output_dir)
}

#[tauri::command]
async fn utx_extract_packages(
    app: tauri::AppHandle,
    file_paths: Vec<String>,
    output_dir: String,
    mode: utx::UtxExtractMode,
) -> Result<utx::UtxExtractSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        utx::extract_packages_with_progress(file_paths, &output_dir, mode, |progress| {
            let _ = app.emit("utx-extract-progress", progress);
        })
    })
    .await
    .map_err(|error| format!("A extração em segundo plano falhou: {error}"))?
}

#[tauri::command]
fn utx_preview_texture(
    file_path: String,
    export_index: usize,
) -> Result<utx::TexturePreview, String> {
    utx::preview_texture(&file_path, export_index)
}

#[tauri::command]
fn utx_replace_entry(
    file_path: String,
    export_index: usize,
    replacement_path: String,
) -> Result<(), String> {
    utx::replace_entry(&file_path, export_index, &replacement_path)
}

#[tauri::command]
fn utx_import_entries(
    file_path: String,
    replacements: Vec<utx::ReplacementRequest>,
) -> Result<utx::ImportSummary, String> {
    utx::import_entries(&file_path, replacements)
}

#[tauri::command]
fn utx_open_cached(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
) -> Result<Vec<utx::UtxEntry>, String> {
    let (session, entries) = utx::open_cached(&file_path)?;
    *cache
        .0
        .lock()
        .map_err(|_| "Não foi possível acessar o cache do pacote UTX.".to_string())? =
        Some(session);
    Ok(entries)
}

#[tauri::command]
fn utx_cached_list_entries(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
) -> Result<Vec<utx::UtxEntry>, String> {
    let guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_ref().ok_or("Nenhum pacote UTX está aberto.")?;
    utx::cached_list_entries(session)
}

#[tauri::command]
fn utx_cached_export_entry(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    export_index: usize,
    output_path: String,
) -> Result<(), String> {
    let guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_ref().ok_or("Nenhum pacote UTX está aberto.")?;
    utx::cached_export_entry(session, export_index, &output_path)
}

#[tauri::command]
async fn utx_cached_export_entries(
    app: tauri::AppHandle,
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    export_indices: Vec<usize>,
    output_dir: String,
) -> Result<utx::ExportSummary, String> {
    let cache = Arc::clone(&cache.0);
    tauri::async_runtime::spawn_blocking(move || {
        let guard = cache
            .lock()
            .map_err(|_| "Não foi possível acessar o cache do pacote UTX.".to_string())?;
        match guard.as_ref() {
            Some(session) if session.matches_path(&file_path) => {}
            Some(_) => {
                return Err("O pacote UTX aberto foi alterado. Abra o arquivo novamente.".into())
            }
            None => return Err("Nenhum pacote UTX está aberto. Abra o arquivo novamente.".into()),
        }
        let session = guard.as_ref().ok_or("Nenhum pacote UTX está aberto.")?;
        utx::cached_export_entries_with_progress(session, export_indices, &output_dir, |progress| {
            let _ = app.emit("utx-export-progress", progress);
        })
    })
    .await
    .map_err(|error| format!("A exportação em segundo plano falhou: {error}"))?
}

#[tauri::command]
fn utx_cached_preview_texture(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    export_index: usize,
) -> Result<utx::TexturePreview, String> {
    let guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_ref().ok_or("Nenhum pacote UTX está aberto.")?;
    utx::cached_preview_texture(session, export_index)
}

#[tauri::command]
fn utx_cached_texture_properties(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    export_index: usize,
) -> Result<texture_engine::TextureEditorState, String> {
    let guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_ref().ok_or("Nenhum pacote UTX está aberto.")?;
    utx::cached_texture_properties(session, export_index)
}

#[tauri::command]
fn utx_cached_update_texture_properties(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    export_index: usize,
    edit: texture_engine::TextureEditorEdit,
) -> Result<(), String> {
    let mut guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_mut().ok_or("Nenhum pacote UTX está aberto.")?;
    utx::cached_update_texture_properties(session, export_index, edit)
}

#[tauri::command]
fn utx_cached_update_texture_properties_batch(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    edits: Vec<UtxTexturePropertiesBatchEdit>,
) -> Result<(), String> {
    let mut guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_mut().ok_or("Nenhum pacote UTX está aberto.")?;
    let edits = edits
        .into_iter()
        .map(|change| (change.export_index, change.edit))
        .collect::<Vec<_>>();
    if edits.is_empty() {
        return Err("Selecione ao menos uma textura para atualizar.".into());
    }
    utx::cached_update_texture_properties_batch(session, &edits)
}

#[tauri::command]
fn utx_cached_duplicate_texture(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    source_export_index: usize,
    group_name: String,
    texture_name: String,
) -> Result<usize, String> {
    let mut guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_mut().ok_or("Nenhum pacote UTX está aberto.")?;
    utx::cached_duplicate_texture(session, source_export_index, &group_name, &texture_name)
}

#[tauri::command]
fn utx_cached_rename_texture(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    export_index: usize,
    texture_name: String,
) -> Result<(), String> {
    let mut guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_mut().ok_or("Nenhum pacote UTX está aberto.")?;
    utx::cached_rename_texture(session, export_index, &texture_name)
}

#[tauri::command]
fn utx_cached_replace_entry(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    export_index: usize,
    replacement_path: String,
) -> Result<(), String> {
    let mut guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_mut().ok_or("Nenhum pacote UTX está aberto.")?;
    utx::cached_replace_entry(session, export_index, &replacement_path)
}

#[tauri::command]
fn utx_cached_import_entries(
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    replacements: Vec<utx::ReplacementRequest>,
) -> Result<utx::ImportSummary, String> {
    let mut guard = cached_utx(&cache, &file_path)?;
    let session = guard.as_mut().ok_or("Nenhum pacote UTX está aberto.")?;
    utx::cached_import_entries(session, replacements)
}

#[tauri::command]
async fn utx_cached_import_textures(
    app: tauri::AppHandle,
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    package_name: String,
    texture_paths: Vec<String>,
) -> Result<utx::TextureImportSummary, String> {
    let cache = Arc::clone(&cache.0);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = cache
            .lock()
            .map_err(|_| "Não foi possível acessar o cache do pacote UTX.".to_string())?;
        match guard.as_ref() {
            Some(session) if session.matches_path(&file_path) => {}
            Some(_) => {
                return Err("O pacote UTX aberto foi alterado. Abra o arquivo novamente.".into());
            }
            None => return Err("Nenhum pacote UTX está aberto. Abra o arquivo novamente.".into()),
        }
        let session = guard.as_mut().ok_or("Nenhum pacote UTX está aberto.")?;
        utx::cached_import_textures_with_progress(
            session,
            &package_name,
            texture_paths,
            |progress| {
                let _ = app.emit("utx-import-progress", progress);
            },
        )
    })
    .await
    .map_err(|error| format!("A importação em segundo plano falhou: {error}"))?
}

#[tauri::command]
async fn utx_cached_import_texture_directory(
    app: tauri::AppHandle,
    cache: tauri::State<'_, UtxCache>,
    file_path: String,
    directory: String,
) -> Result<utx::TextureImportSummary, String> {
    let cache = Arc::clone(&cache.0);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = cache
            .lock()
            .map_err(|_| "Não foi possível acessar o cache do pacote UTX.".to_string())?;
        match guard.as_ref() {
            Some(session) if session.matches_path(&file_path) => {}
            Some(_) => {
                return Err("O pacote UTX aberto foi alterado. Abra o arquivo novamente.".into());
            }
            None => return Err("Nenhum pacote UTX está aberto. Abra o arquivo novamente.".into()),
        }
        let session = guard.as_mut().ok_or("Nenhum pacote UTX está aberto.")?;
        utx::cached_import_texture_directory_with_progress(session, &directory, |progress| {
            let _ = app.emit("utx-import-progress", progress);
        })
    })
    .await
    .map_err(|error| format!("A importação em segundo plano falhou: {error}"))?
}

#[tauri::command]
fn utx_clear_cache(cache: tauri::State<'_, UtxCache>) -> Result<(), String> {
    *cache
        .0
        .lock()
        .map_err(|_| "Não foi possível liberar o cache do pacote UTX.".to_string())? = None;
    Ok(())
}

#[tauri::command]
async fn texture_resize_directory(
    app: tauri::AppHandle,
    directory: String,
    source_resolution: u32,
    target_resolution: u32,
) -> Result<texture_resize::ResizeSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        texture_resize::resize_directory_with_progress(
            &directory,
            source_resolution,
            target_resolution,
            |progress| {
                let _ = app.emit("texture-resize-progress", progress);
            },
        )
    })
    .await
    .map_err(|error| format!("O redimensionamento em segundo plano falhou: {error}"))?
}

#[tauri::command]
async fn geodata_convert_directory(
    app: tauri::AppHandle,
    input_directory: String,
    output_directory: String,
    target_format: geodata::GeodataFormat,
) -> Result<geodata::GeodataSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        geodata::convert_directory_with_progress(
            &input_directory,
            &output_directory,
            target_format,
            |progress| {
                let _ = app.emit("geodata-convert-progress", progress);
            },
        )
    })
    .await
    .map_err(|error| format!("A conversão geodata em segundo plano falhou: {error}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(UtxCache(Arc::new(Mutex::new(None))))
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview.window().show();
            }
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            app_settings_load,
            app_settings_save,
            app_settings_open_directory,
            app_logs_open_directory,
            utx_list_entries,
            utx_create_new,
            utx_export_entry,
            utx_export_entries,
            utx_extract_packages,
            utx_preview_texture,
            utx_replace_entry,
            utx_import_entries,
            utx_open_cached,
            utx_cached_list_entries,
            utx_cached_export_entry,
            utx_cached_export_entries,
            utx_cached_preview_texture,
            utx_cached_texture_properties,
            utx_cached_update_texture_properties,
            utx_cached_update_texture_properties_batch,
            utx_cached_duplicate_texture,
            utx_cached_rename_texture,
            utx_cached_replace_entry,
            utx_cached_import_entries,
            utx_cached_import_textures,
            utx_cached_import_texture_directory,
            utx_clear_cache,
            texture_resize_directory,
            geodata_convert_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
