use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use evdev::KeyCode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Command {
    Run(RunConfig),
    LearnBinding(LearnBindingConfig),
    Setup(SetupConfig),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RunConfig {
    pub device_name: String,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LearnBindingConfig {
    pub binding: LearnedBinding,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SetupConfig {
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LearnedBinding {
    Trigger,
    SideA,
    SideB,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistedConfig {
    #[serde(default)]
    pub bindings: BindingConfig,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BindingConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_a: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_b: Option<String>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            device_name: String::from("fun-mouse"),
            config_path: std::env::var_os("FUN_MOUSE_CONFIG")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("XDG_CONFIG_HOME")
                        .map(PathBuf::from)
                        .map(|xdg| xdg.join("fun-mouse").join("config.toml"))
                })
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .map(|home| home.join(".config").join("fun-mouse").join("config.toml"))
                })
                .unwrap_or_else(|| PathBuf::from("fun-mouse.toml")),
        }
    }
}

impl Command {
    pub fn from_env_args() -> Result<Self> {
        Self::from_args(std::env::args().skip(1))
    }

    pub fn from_args<I, S>(args: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let args = args.into_iter().map(Into::into).collect::<Vec<String>>();

        if matches!(args.first().map(String::as_str), Some("learn-binding")) {
            return parse_learn_binding_args(args.into_iter().skip(1));
        }
        if matches!(args.first().map(String::as_str), Some("setup")) {
            return parse_setup_args(args.into_iter().skip(1));
        }

        parse_run_args(args.into_iter())
    }

    pub fn usage() -> &'static str {
        "usage:\n  fun-mouse [--device-name <name>] [--config <path>]\n  fun-mouse setup [--config <path>]\n  fun-mouse learn-binding <trigger|side-a|side-b> [--config <path>]\n\nnotes:\n  F9 toggles relay on/off.\n  off = ungrab physical mice and return control to the system.\n  setup asks for trigger only and saves the BTN_* binding.\n  learn-binding grabs detected mice temporarily, waits for the first BTN_* press,\n  prints it and saves it into a toml config.\n"
    }
}

impl LearnedBinding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trigger => "trigger",
            Self::SideA => "side-a",
            Self::SideB => "side-b",
        }
    }
}

impl std::str::FromStr for LearnedBinding {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "trigger" => Ok(Self::Trigger),
            "side-a" | "side_a" => Ok(Self::SideA),
            "side-b" | "side_b" => Ok(Self::SideB),
            other => bail!("unsupported binding name `{other}`"),
        }
    }
}

impl PersistedConfig {
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let encoded = toml::to_string_pretty(self)?;
        fs::write(path, encoded)?;
        Ok(())
    }

    pub fn set_binding(&mut self, binding: LearnedBinding, key_name: String) {
        match binding {
            LearnedBinding::Trigger => self.bindings.trigger = Some(key_name),
            LearnedBinding::SideA => self.bindings.side_a = Some(key_name),
            LearnedBinding::SideB => self.bindings.side_b = Some(key_name),
        }
    }
}

pub fn parse_mouse_button_key(name: &str) -> Result<KeyCode> {
    match name.trim().to_ascii_uppercase().as_str() {
        "BTN_LEFT" => Ok(KeyCode::BTN_LEFT),
        "BTN_RIGHT" => Ok(KeyCode::BTN_RIGHT),
        "BTN_MIDDLE" => Ok(KeyCode::BTN_MIDDLE),
        "BTN_SIDE" => Ok(KeyCode::BTN_SIDE),
        "BTN_EXTRA" => Ok(KeyCode::BTN_EXTRA),
        other => bail!("unsupported mouse button binding `{other}`"),
    }
}

fn parse_run_args<I>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = String>,
{
    let mut cfg = RunConfig::default();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--device-name" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --device-name"))?;
                let value = value.trim();
                if value.is_empty() {
                    bail!("--device-name must not be empty");
                }
                cfg.device_name = value.to_string();
            }
            "--config" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --config"))?;
                let value = value.trim();
                if value.is_empty() {
                    bail!("--config must not be empty");
                }
                cfg.config_path = PathBuf::from(value);
            }
            "--help" | "-h" => {
                print!("{}", Command::usage());
                std::process::exit(0);
            }
            other => bail!("unsupported argument `{other}`\n\n{}", Command::usage()),
        }
    }

    Ok(Command::Run(cfg))
}

fn parse_learn_binding_args<I>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let binding = args
        .next()
        .ok_or_else(|| anyhow!("missing binding name for learn-binding"))?
        .parse()?;
    let mut config_path = std::env::var_os("FUN_MOUSE_CONFIG")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .map(|xdg| xdg.join("fun-mouse").join("config.toml"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config").join("fun-mouse").join("config.toml"))
        })
        .unwrap_or_else(|| PathBuf::from("fun-mouse.toml"));

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --config"))?;
                let value = value.trim();
                if value.is_empty() {
                    bail!("--config must not be empty");
                }
                config_path = PathBuf::from(value);
            }
            "--help" | "-h" => {
                print!("{}", Command::usage());
                std::process::exit(0);
            }
            other => bail!(
                "unsupported learn-binding argument `{other}`\n\n{}",
                Command::usage()
            ),
        }
    }

    Ok(Command::LearnBinding(LearnBindingConfig {
        binding,
        config_path,
    }))
}

fn parse_setup_args<I>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let mut config_path = std::env::var_os("FUN_MOUSE_CONFIG")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .map(|xdg| xdg.join("fun-mouse").join("config.toml"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config").join("fun-mouse").join("config.toml"))
        })
        .unwrap_or_else(|| PathBuf::from("fun-mouse.toml"));

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --config"))?;
                let value = value.trim();
                if value.is_empty() {
                    bail!("--config must not be empty");
                }
                config_path = PathBuf::from(value);
            }
            "--help" | "-h" => {
                print!("{}", Command::usage());
                std::process::exit(0);
            }
            other => bail!(
                "unsupported setup argument `{other}`\n\n{}",
                Command::usage()
            ),
        }
    }

    Ok(Command::Setup(SetupConfig { config_path }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Command, LearnedBinding, PersistedConfig, parse_mouse_button_key};
    use evdev::KeyCode;

    #[test]
    fn config_defaults_are_minimal() {
        let Command::Run(cfg) = Command::from_args(Vec::<String>::new()).expect("config") else {
            panic!("expected run config");
        };
        assert_eq!(cfg.device_name, "fun-mouse");
        assert!(cfg.config_path.ends_with("config.toml"));
    }

    #[test]
    fn config_parses_device_name() {
        let Command::Run(cfg) = Command::from_args(["--device-name", "mfc-mouse"]).expect("config")
        else {
            panic!("expected run config");
        };
        assert_eq!(cfg.device_name, "mfc-mouse");
    }

    #[test]
    fn config_parses_run_config_path() {
        let Command::Run(cfg) = Command::from_args(["--config", "/tmp/fun.toml"]).expect("config")
        else {
            panic!("expected run config");
        };
        assert_eq!(cfg.config_path, PathBuf::from("/tmp/fun.toml"));
    }

    #[test]
    fn config_parses_learn_binding_mode() {
        let Command::LearnBinding(cfg) =
            Command::from_args(["learn-binding", "trigger", "--config", "/tmp/fun.toml"])
                .expect("config")
        else {
            panic!("expected learn-binding config");
        };
        assert_eq!(cfg.binding, LearnedBinding::Trigger);
        assert_eq!(cfg.config_path, Path::new("/tmp/fun.toml"));
    }

    #[test]
    fn config_parses_setup_mode() {
        let Command::Setup(cfg) =
            Command::from_args(["setup", "--config", "/tmp/fun.toml"]).expect("config")
        else {
            panic!("expected setup config");
        };
        assert_eq!(cfg.config_path, Path::new("/tmp/fun.toml"));
    }

    #[test]
    fn persisted_config_sets_binding_value() {
        let mut config = PersistedConfig::default();
        config.set_binding(LearnedBinding::Trigger, String::from("BTN_EXTRA"));
        assert_eq!(config.bindings.trigger.as_deref(), Some("BTN_EXTRA"));
        assert_eq!(config.bindings.side_a, None);
        assert_eq!(config.bindings.side_b, None);
    }

    #[test]
    fn parse_mouse_button_key_supports_side_buttons() {
        assert_eq!(
            parse_mouse_button_key("BTN_SIDE").expect("parse"),
            KeyCode::BTN_SIDE
        );
        assert_eq!(
            parse_mouse_button_key("btn_extra").expect("parse"),
            KeyCode::BTN_EXTRA
        );
    }
}
