//! Representation and loading for config structures
use serde::Deserialize;

use std::path::Path;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub rules: Vec<Rule>,
}

impl Config {
    fn load_from_path<P: AsRef<Path>>(pth: P) -> Option<Self> {
        let input = std::fs::File::open(pth).ok()?;
        let input = std::io::BufReader::new(input);
        serde_json::from_reader(input).ok()
    }

    pub(crate) fn rules_matching<'a, 'e: 'a>(
        &'a self,
        entry: &'e super::DeviceEntry
    ) -> impl Iterator<Item=&Rule> + 'a {
        self.rules
            .as_slice()
            .into_iter()
            .filter(|r| r.criteria.matches(entry))
    }
}

#[derive(Deserialize, Debug)]
pub struct Rule {
    #[serde(flatten)]
    pub criteria: Criteria,
    pub priority: Option<i64>,

    #[serde(default)]
    pub hide: bool,
}

#[derive(Deserialize, Debug)]
pub struct Criteria {
    #[serde(default)]
    pub invert: bool,

    pub card_name: Option<String>,
    pub is_display: Option<bool>,
}

impl Criteria {
    fn matches(&self, entry: &super::DeviceEntry) -> bool {
        if let Some(name) = self.card_name.as_ref() {
            if name != &entry.name {
                return false;
            }
        }

        if let Some(is_disp) = self.is_display.as_ref() {
            if (*is_disp) != !entry.displays.is_empty() {
                return false;
            }
        }

        return true;
    }
}

impl Default for Config {
    fn default() -> Self {
        Self { rules: Vec::new() }
    }
}

fn get_config() -> Config {
    std::env::var_os("VK_REORDER_CONFIG")
             .and_then(Config::load_from_path)
             .or_else(|| Config::load_from_path("vk_device_reorder.json"))
             .or_else(|| Config::load_from_path("/etc/vk_device_reorder.json"))
             .unwrap_or_else(Config::default)
}

lazy_static::lazy_static! {
    pub(crate) static ref CONFIG: Config = get_config();
}
