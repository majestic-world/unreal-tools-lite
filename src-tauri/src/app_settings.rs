use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub type AppSettings = BTreeMap<String, String>;

const SETTINGS_FILE_NAME: &str = "settings.json";

pub fn settings_file(app_data_directory: &Path) -> PathBuf {
    app_data_directory.join(SETTINGS_FILE_NAME)
}

pub fn load(app_data_directory: &Path) -> Result<AppSettings, String> {
    let settings_file = settings_file(app_data_directory);
    match fs::read(&settings_file) {
        Ok(contents) => serde_json::from_slice(&contents).map_err(|error| {
            format!(
                "Não foi possível ler as configurações em '{}': {error}",
                settings_file.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::new()),
        Err(error) => Err(format!(
            "Não foi possível abrir as configurações em '{}': {error}",
            settings_file.display()
        )),
    }
}

pub fn save(app_data_directory: &Path, settings: &AppSettings) -> Result<(), String> {
    fs::create_dir_all(app_data_directory).map_err(|error| {
        format!(
            "Não foi possível criar a pasta de configurações em '{}': {error}",
            app_data_directory.display()
        )
    })?;

    let settings_file = settings_file(app_data_directory);
    let temporary_file = app_data_directory.join("settings.json.tmp");
    let serialized = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Não foi possível preparar as configurações: {error}"))?;

    fs::write(&temporary_file, serialized).map_err(|error| {
        format!(
            "Não foi possível salvar as configurações temporárias em '{}': {error}",
            temporary_file.display()
        )
    })?;

    if settings_file.exists() {
        fs::remove_file(&settings_file).map_err(|error| {
            format!(
                "Não foi possível atualizar as configurações em '{}': {error}",
                settings_file.display()
            )
        })?;
    }

    fs::rename(&temporary_file, &settings_file).map_err(|error| {
        format!(
            "Não foi possível finalizar as configurações em '{}': {error}",
            settings_file.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{load, save, settings_file, AppSettings};
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temporary_directory() -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        env::temp_dir().join(format!("unreal-tools-settings-{suffix}"))
    }

    #[test]
    fn missing_settings_file_returns_an_empty_map() {
        let directory = temporary_directory();
        assert_eq!(
            load(&directory).expect("load should succeed"),
            AppSettings::new()
        );
    }

    #[test]
    fn settings_round_trip_through_json_file() {
        let directory = temporary_directory();
        let mut expected = AppSettings::new();
        expected.insert("unreal-tools.appearance.theme".into(), "dark".into());
        expected.insert(
            "unreal-tools.utx.last-open-directory".into(),
            "C:\\Textures".into(),
        );

        save(&directory, &expected).expect("save should succeed");

        assert!(settings_file(&directory).is_file());
        assert_eq!(load(&directory).expect("load should succeed"), expected);
        fs::remove_dir_all(&directory).expect("temporary directory should be removable");
    }
}
