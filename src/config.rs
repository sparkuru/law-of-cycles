use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::{collections::HashMap, env, fs, io::Read, path::PathBuf, time::Duration};
use url::Url;

#[derive(Clone)]
pub struct Settings {
    pub controller: Url,
    pub secret: String,
    pub timeout: Duration,
    pub connections: ConnectionSettings,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionSettings {
    pub show_process: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSettings {
    controller: Option<String>,
    secret: Option<String>,
    timeout: Option<f64>,
    #[serde(default)]
    connections: ConnectionSettings,
}

#[derive(Deserialize)]
struct MihomoSettings {
    #[serde(rename = "external-controller")]
    controller: Option<String>,
    #[serde(rename = "external-controller-tls")]
    controller_tls: Option<String>,
    secret: Option<String>,
}

fn mihomo_settings(text: &str) -> Result<FileSettings> {
    // Parser errors may contain source excerpts, including credentials.
    let file: MihomoSettings = serde_saphyr::from_str(text).map_err(|_| {
        anyhow::anyhow!("Invalid Mihomo YAML; controller and secret must be strings")
    })?;
    let address = file
        .controller_tls
        .filter(|value| !value.is_empty())
        .map(|value| ("https", value))
        .or_else(|| {
            file.controller
                .filter(|value| !value.is_empty())
                .map(|value| ("http", value))
        });
    let controller = address
        .map(|(scheme, address)| controller_origin(scheme, &address))
        .transpose()?;
    Ok(FileSettings {
        controller,
        secret: file.secret,
        timeout: None,
        connections: ConnectionSettings::default(),
    })
}

fn controller_origin(scheme: &str, address: &str) -> Result<String> {
    let (host, port) = address
        .rsplit_once(':')
        .context("Mihomo controller must be a host:port listener")?;
    ensure!(
        port.parse::<u16>().is_ok_and(|port| port > 0),
        "Invalid Mihomo controller port"
    );
    let host = match host {
        "" | "*" | "0.0.0.0" => "127.0.0.1",
        "[::]" => "[::1]",
        value => value,
    };
    let controller = format!("{scheme}://{host}:{port}");
    Settings::new(&controller, String::new(), 5.0)?;
    Ok(controller)
}

impl Settings {
    pub fn new(controller: &str, secret: String, seconds: f64) -> Result<Self> {
        ensure!(
            !controller
                .chars()
                .any(|c| c.is_control() || c.is_whitespace()),
            "Invalid controller URL"
        );
        let url = Url::parse(controller).map_err(|_| anyhow::anyhow!("Invalid controller URL"))?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && matches!(url.path(), "" | "/")
                && url.port() != Some(0),
            "Controller must be an HTTP(S) origin without credentials or a path"
        );
        ensure!(
            seconds.is_finite() && seconds > 0.0 && seconds <= 120.0,
            "Timeout must be between 0 and 120 seconds"
        );
        ensure!(
            secret.bytes().all(|byte| (32..=126).contains(&byte)),
            "Secret must contain printable ASCII only"
        );
        Ok(Self {
            controller: url,
            secret,
            timeout: Duration::from_secs_f64(seconds),
            connections: ConnectionSettings::default(),
        })
    }

    pub fn load(
        path: Option<PathBuf>,
        mihomo_path: Option<PathBuf>,
        controller: Option<String>,
        timeout: Option<f64>,
        require_controller: bool,
    ) -> Result<Self> {
        Self::load_with(
            path,
            mihomo_path,
            controller,
            timeout,
            &env::vars().collect(),
            require_controller,
        )
    }

    fn load_with(
        path: Option<PathBuf>,
        mihomo_path: Option<PathBuf>,
        controller: Option<String>,
        timeout: Option<f64>,
        env: &HashMap<String, String>,
        require_controller: bool,
    ) -> Result<Self> {
        ensure!(
            path.is_none() || mihomo_path.is_none(),
            "Choose either --config or --mihomo-config"
        );
        let explicit = path.is_some() || mihomo_path.is_some();
        let force_yaml = mihomo_path.is_some();
        let path = mihomo_path.or(path).unwrap_or_else(|| {
            let root = env
                .get("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env.get("HOME").map(String::as_str).unwrap_or("."))
                        .join(".config")
                });
            root.join("kami/config.toml")
        });
        let yaml = force_yaml
            || path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| {
                    value.eq_ignore_ascii_case("yaml") || value.eq_ignore_ascii_case("yml")
                });
        let file: FileSettings = match fs::File::open(path) {
            Ok(file) => {
                let mut text = String::new();
                file.take(16 * 1024 * 1024 + 1)
                    .read_to_string(&mut text)
                    .map_err(|_| anyhow::anyhow!("Cannot read configuration as UTF-8"))?;
                ensure!(
                    text.len() <= 16 * 1024 * 1024,
                    "Configuration exceeds 16 MiB"
                );
                if yaml {
                    mihomo_settings(&text)?
                } else {
                    toml::from_str(&text).map_err(|_| {
                        anyhow::anyhow!(
                            "Invalid configuration TOML; expected controller, secret, timeout, connections"
                        )
                    })?
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !explicit => {
                FileSettings::default()
            }
            Err(_) => bail!("Cannot read requested configuration file"),
        };
        let controller = controller
            .or_else(|| env.get("KAMI_CONTROLLER").cloned())
            .or(file.controller);
        ensure!(
            !require_controller || controller.is_some(),
            "TUI needs a controller; pass --controller URL (set KAMI_SECRET if required), or --config PATH with a controller"
        );
        ensure!(
            !yaml || controller.is_some(),
            "Mihomo YAML has no HTTP(S) controller; configure external-controller or pass --controller (Unix sockets are not supported)"
        );
        let controller = controller.unwrap_or_else(|| "http://127.0.0.1:9090".into());
        let secret = env
            .get("KAMI_SECRET")
            .cloned()
            .or(file.secret)
            .unwrap_or_default();
        let seconds = match timeout {
            Some(value) => value,
            None => match env.get("KAMI_TIMEOUT") {
                Some(value) => value.parse().context("KAMI_TIMEOUT must be a number")?,
                None => file.timeout.unwrap_or(5.0),
            },
        };
        let mut settings = Self::new(&controller, secret, seconds)?;
        settings.connections = file.connections;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ConfigFile(PathBuf);
    impl ConfigFile {
        fn new(extension: &str, text: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = env::temp_dir().join(format!(
                "kami-config-{}-{}.{extension}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::write(&path, text).unwrap();
            Self(path)
        }
    }
    impl Drop for ConfigFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn yaml_extracts_connection_and_ignores_proxy_rules() {
        let file = ConfigFile::new(
            "yaml",
            "external-controller: 0.0.0.0:9090\nsecret: 'test # secret'\nproxies: []\nrules: [MATCH,DIRECT]\n",
        );
        let settings = Settings::load_with(
            Some(file.0.clone()),
            None,
            None,
            None,
            &HashMap::new(),
            false,
        )
        .unwrap();
        assert_eq!(settings.controller.as_str(), "http://127.0.0.1:9090/");
        assert_eq!(settings.secret, "test # secret");
        assert_eq!(settings.timeout.as_secs(), 5);
    }

    #[test]
    fn yaml_listener_normalization_and_tls_preference() {
        for (address, expected) in [
            (":9090", "127.0.0.1"),
            ("*:9090", "127.0.0.1"),
            ("[::]:9090", "[::1]"),
            ("[::1]:9090", "[::1]"),
            ("192.0.2.1:9090", "192.0.2.1"),
        ] {
            assert_eq!(
                controller_origin("http", address).unwrap(),
                format!("http://{expected}:9090")
            );
        }
        let file = mihomo_settings(
            "external-controller: ':9090'\nexternal-controller-tls: 'example.com:9443'\n",
        )
        .unwrap();
        assert_eq!(file.controller.as_deref(), Some("https://example.com:9443"));
        for address in [
            "localhost",
            "localhost:0",
            "http://localhost:9090",
            "user:pass@localhost:9090",
            "localhost/path:9090",
            "localhost:65536",
        ] {
            assert!(controller_origin("http", address).is_err());
        }
    }

    #[test]
    fn explicit_yaml_file_supports_any_extension_and_overrides() {
        let file = ConfigFile::new(
            "conf",
            "external-controller: ':9090'\nsecret: file-secret\n",
        );
        let env = HashMap::from([
            ("KAMI_CONTROLLER".into(), "http://localhost:2222".into()),
            ("KAMI_SECRET".into(), "env-secret".into()),
            ("KAMI_TIMEOUT".into(), "4".into()),
        ]);
        let settings = Settings::load_with(
            None,
            Some(file.0.clone()),
            Some("http://localhost:3333".into()),
            Some(6.0),
            &env,
            false,
        )
        .unwrap();
        assert_eq!(settings.controller.port(), Some(3333));
        assert_eq!(settings.secret, "env-secret");
        assert_eq!(settings.timeout.as_secs(), 6);
        let settings =
            Settings::load_with(None, Some(file.0.clone()), None, None, &env, false).unwrap();
        assert_eq!(settings.controller.port(), Some(2222));
        assert_eq!(settings.timeout.as_secs(), 4);
    }

    #[test]
    fn yaml_without_listener_needs_an_explicit_target() {
        let file = ConfigFile::new(
            "yml",
            "external-controller-unix: /tmp/mihomo.sock\nsecret: test-secret\n",
        );
        assert!(
            Settings::load_with(
                Some(file.0.clone()),
                None,
                None,
                None,
                &HashMap::new(),
                false
            )
            .is_err()
        );
        let settings = Settings::load_with(
            Some(file.0.clone()),
            None,
            Some("http://localhost:9090".into()),
            None,
            &HashMap::new(),
            false,
        )
        .unwrap();
        assert_eq!(settings.secret, "test-secret");
    }

    #[test]
    fn yaml_errors_do_not_echo_secrets() {
        for text in [
            "secret: [private-marker",
            "secret: {private-marker: value}",
            "external-controller: [private-marker]",
            "- private-marker",
        ] {
            let error = mihomo_settings(text).err().expect("invalid YAML accepted");
            assert!(!format!("{error:#}").contains("private-marker"));
        }
        let file = mihomo_settings(
            "x-address: &api '127.0.0.1:9090'\nexternal-controller: *api\nsecret: ''\n",
        )
        .unwrap();
        assert_eq!(file.controller.as_deref(), Some("http://127.0.0.1:9090"));
        assert_eq!(file.secret.as_deref(), Some(""));
    }

    #[test]
    fn toml_compatibility_is_preserved() {
        let file = ConfigFile::new(
            "toml",
            "controller = 'http://localhost:9091'\nsecret = 'toml-secret'\ntimeout = 3\n",
        );
        let settings = Settings::load_with(
            Some(file.0.clone()),
            None,
            None,
            None,
            &HashMap::new(),
            false,
        )
        .unwrap();
        assert_eq!(settings.controller.port(), Some(9091));
        assert_eq!(settings.secret, "toml-secret");
        assert_eq!(settings.timeout.as_secs(), 3);
        assert!(!settings.connections.show_process);
    }
    #[test]
    fn connection_process_column_is_opt_in() {
        let file = ConfigFile::new("toml", "[connections]\nshow_process = true\n");
        let settings = Settings::load_with(
            Some(file.0.clone()),
            None,
            None,
            None,
            &HashMap::new(),
            false,
        )
        .unwrap();
        assert!(settings.connections.show_process);
        assert!(toml::from_str::<FileSettings>("[connections]\nshow_process = 'true'\n").is_err());
        assert!(toml::from_str::<FileSettings>("[connections]\nprocess = true\n").is_err());
    }
    #[test]
    fn rejects_invalid_controller_and_header_injection() {
        for url in [
            "file:///tmp/x",
            "http://user:pass@localhost",
            "http://localhost/path",
            "http://localhost?token=x",
            "http://localhost:0",
            "http://local\nhost",
        ] {
            assert!(Settings::new(url, String::new(), 5.0).is_err(), "{url}");
        }
        assert!(Settings::new("http://localhost", "a\r\nX: y".into(), 5.0).is_err());
        for seconds in [0.0, -1.0, f64::NAN, f64::INFINITY, 121.0] {
            assert!(Settings::new("http://localhost", String::new(), seconds).is_err());
        }
    }
    #[test]
    fn flags_override_environment_and_defaults() {
        let env = HashMap::from([
            ("XDG_CONFIG_HOME".into(), "/nonexistent-kami-test".into()),
            ("KAMI_CONTROLLER".into(), "http://localhost:2222".into()),
            ("KAMI_SECRET".into(), "test".into()),
            ("KAMI_TIMEOUT".into(), "4".into()),
        ]);
        let settings = Settings::load_with(
            None,
            None,
            Some("http://localhost:3333".into()),
            Some(6.0),
            &env,
            false,
        )
        .unwrap();
        assert_eq!(settings.controller.port(), Some(3333));
        assert_eq!(settings.timeout.as_secs(), 6);
        assert_eq!(settings.secret, "test");
    }

    #[test]
    fn tui_requires_a_configured_controller() {
        let file = ConfigFile::new("toml", "secret = 'file-secret'\n");
        let error = Settings::load_with(
            Some(file.0.clone()),
            None,
            None,
            None,
            &HashMap::new(),
            true,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("TUI needs a controller"));

        let settings = Settings::load_with(
            Some(file.0.clone()),
            None,
            Some("http://localhost:9090".into()),
            None,
            &HashMap::new(),
            true,
        )
        .unwrap();
        assert_eq!(settings.secret, "file-secret");

        let env = HashMap::from([("KAMI_CONTROLLER".into(), "http://localhost:9091".into())]);
        assert!(Settings::load_with(None, None, None, None, &env, true).is_ok());
    }
}
