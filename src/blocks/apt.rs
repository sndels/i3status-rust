use std::env;
use std::fs;
use std::io::Write;
use std::process::Command;
use std::time::Duration;

use crossbeam_channel::Sender;
use lazy_static::lazy_static;
use regex::Regex;
use serde_derive::Deserialize;

use crate::blocks::{Block, ConfigBlock, Update};
use crate::config::SharedConfig;
use crate::de::deserialize_duration;
use crate::errors::*;
use crate::formatting::value::Value;
use crate::formatting::FormatTemplate;
use crate::protocol::i3bar_event::{I3BarEvent, MouseButton};
use crate::scheduler::Task;
use crate::widgets::text::TextWidget;
use crate::widgets::{I3BarWidget, State};

pub struct Apt {
    output: TextWidget,
    update_interval: Duration,
    format: FormatTemplate,
    format_singular: FormatTemplate,
    format_up_to_date: FormatTemplate,
    warning_updates_regex: Option<Regex>,
    critical_updates_regex: Option<Regex>,
    config_path: String,
    ignore_waiting_phased_updates: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields, default)]
pub struct AptConfig {
    /// Update interval in seconds
    #[serde(deserialize_with = "deserialize_duration")]
    pub interval: Duration,

    /// Format override
    pub format: FormatTemplate,

    /// Alternative format override for when exactly 1 update is available
    pub format_singular: FormatTemplate,

    /// Alternative format override for when no updates are available
    pub format_up_to_date: FormatTemplate,

    /// Indicate a `warning` state for the block if any pending update match the
    /// following regex. Default behaviour is that no package updates are deemed
    /// warning
    pub warning_updates_regex: Option<String>,

    /// Indicate a `critical` state for the block if any pending update match the following regex.
    /// Default behaviour is that no package updates are deemed critical
    pub critical_updates_regex: Option<String>,

    /// Removes phased updates under 100% from the update count
    pub ignore_waiting_phased_updates: bool,
}

impl Default for AptConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(600),
            format: FormatTemplate::default(),
            format_singular: FormatTemplate::default(),
            format_up_to_date: FormatTemplate::default(),
            warning_updates_regex: None,
            critical_updates_regex: None,
            ignore_waiting_phased_updates: false,
        }
    }
}

impl ConfigBlock for Apt {
    type Config = AptConfig;

    fn new(
        id: usize,
        block_config: Self::Config,
        shared_config: SharedConfig,
        _tx_update_request: Sender<Task>,
    ) -> Result<Self> {
        let mut cache_dir = env::temp_dir();
        cache_dir.push("i3rs-apt");
        if !cache_dir.exists() {
            fs::create_dir(&cache_dir).error_msg("Failed to create temp dir")?;
        }

        let apt_conf = format!(
            "Dir::State \"{}\";\n
             Dir::State::lists \"lists\";\n
             Dir::Cache \"{}\";\n
             Dir::Cache::srcpkgcache \"srcpkgcache.bin\";\n
             Dir::Cache::pkgcache \"pkgcache.bin\";",
            cache_dir.display(),
            cache_dir.display()
        );
        cache_dir.push("apt.conf");
        let mut config_file =
            fs::File::create(&cache_dir).error_msg("Failed to create config file")?;
        write!(config_file, "{}", apt_conf).error_msg("Failed to write to config file")?;

        let output = TextWidget::new(id, 0, shared_config).with_icon("update")?;

        Ok(Apt {
            update_interval: block_config.interval,
            format: block_config.format.with_default("{count:1}")?,
            format_singular: block_config.format_singular.with_default("{count:1}")?,
            format_up_to_date: block_config.format_up_to_date.with_default("{count:1}")?,
            output,
            warning_updates_regex: block_config
                .warning_updates_regex
                .as_deref()
                .map(Regex::new)
                .transpose()
                .error_msg("invalid warning updates regex")?,
            critical_updates_regex: block_config
                .critical_updates_regex
                .as_deref()
                .map(Regex::new)
                .transpose()
                .error_msg("invalid critical updates regex")?,
            config_path: cache_dir.into_os_string().into_string().unwrap(),
            ignore_waiting_phased_updates: block_config.ignore_waiting_phased_updates,
        })
    }
}

fn has_warning_update(updates: &str, regex: &Regex) -> bool {
    updates.lines().filter(|line| regex.is_match(line)).count() > 0
}

fn has_critical_update(updates: &str, regex: &Regex) -> bool {
    updates.lines().filter(|line| regex.is_match(line)).count() > 0
}

fn get_updates_list(config_path: &str) -> Result<String> {
    // Update database
    Command::new("sh")
        .env("APT_CONFIG", config_path)
        .args(&["-c", "apt update"])
        .output()
        .error_msg("Failed to run `apt update` command")?;

    String::from_utf8(
        Command::new("sh")
            .env("APT_CONFIG", config_path)
            .args(&["-c", "apt list --upgradable"])
            .output()
            .error_msg("Problem running apt command")?
            .stdout,
    )
    .error_msg("Problem capturing apt command output")
}

fn get_update_count(updates: &str) -> usize {
    updates
        .lines()
        .filter(|line| line.contains("[upgradable"))
        .count()
}

fn get_update_count_ignore_waiting_phased(config_path: &str, updates: &str) -> Result<usize> {
    let non_phased_updates = updates
        .lines()
        .filter(|line| line.contains("[upgradable"))
        .filter_map(|line| match is_waiting_phased_update(config_path, line) {
            Ok(true) => Some(Ok(true)),
            Ok(false) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<Vec<bool>>>()?;

    Ok(non_phased_updates.iter().count())
}

fn is_waiting_phased_update(config_path: &str, package_line: &str) -> Result<bool> {
    lazy_static! {
        static ref PHASED_REGEX: Regex = Regex::new(r#".*\(phased (\d+)%\).*"#).unwrap();
        static ref PACKAGE_NAME_REGEX: Regex = Regex::new(r#"(.*)/.*"#).unwrap();
    }

    let package_name = &PACKAGE_NAME_REGEX
        .captures(package_line)
        .error_msg("Couldn't find package name")?[1];

    let output = String::from_utf8(
        Command::new("sh")
            .env("APT_CONFIG", config_path)
            .args(&["-c", "apt-cache policy", package_name])
            .output()
            .error_msg("Problem running apt-cache command")?
            .stdout,
    )
    .error_msg("Problem capturing apt-cache command output")?;

    Ok(match PHASED_REGEX.captures(&output) {
        Some(matches) => &matches[1] != "100",
        None => false,
    })
}

impl Block for Apt {
    fn name(&self) -> &'static str {
        "apt"
    }

    fn view(&self) -> Vec<&dyn I3BarWidget> {
        vec![&self.output]
    }

    fn update(&mut self) -> Result<Option<Update>> {
        let (formatting_map, warning, critical, cum_count) = {
            let updates_list = get_updates_list(&self.config_path)?;
            let count = if self.ignore_waiting_phased_updates {
                get_update_count_ignore_waiting_phased(&self.config_path, &updates_list)?
            } else {
                get_update_count(&updates_list)
            };
            let formatting_map = map!(
                "count" => Value::from_integer(count as i64)
            );

            let warning = self
                .warning_updates_regex
                .as_ref()
                .map_or(false, |regex| has_warning_update(&updates_list, regex));
            let critical = self
                .critical_updates_regex
                .as_ref()
                .map_or(false, |regex| has_critical_update(&updates_list, regex));

            (formatting_map, warning, critical, count)
        };
        self.output.set_texts(match cum_count {
            0 => self.format_up_to_date.render(&formatting_map)?,
            1 => self.format_singular.render(&formatting_map)?,
            _ => self.format.render(&formatting_map)?,
        });
        self.output.set_state(match cum_count {
            0 => State::Idle,
            _ => {
                if critical {
                    State::Critical
                } else if warning {
                    State::Warning
                } else {
                    State::Info
                }
            }
        });
        Ok(Some(self.update_interval.into()))
    }

    fn click(&mut self, event: &I3BarEvent) -> Result<()> {
        if event.button == MouseButton::Left {
            self.update()?;
        }
        Ok(())
    }
}
