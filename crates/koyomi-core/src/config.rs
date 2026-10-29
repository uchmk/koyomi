//! `koyomi.toml` in the settings folder: how many jobs run at once, how long
//! history is kept, how big one job's log may grow.
//!
//! Missing file: the defaults. A file that does not read is an error the
//! engine shows and does not run with (a typo must not quietly mean "4").

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Jobs running at the same time, over every net.
    pub concurrency: usize,
    /// Days of history and logs kept.
    pub keep_days: u32,
    /// One job's log stops growing here; the rest of its output is not written.
    pub log_max_mb: u64,
}

impl Default for Config {
    fn default() -> Config {
        Config { concurrency: 4, keep_days: 90, log_max_mb: 10 }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(text).map_err(|e| e.message().to_owned())?;
        if c.concurrency == 0 {
            return Err("concurrency は 1 以上".to_owned());
        }
        Ok(c)
    }

    /// The file, or the defaults when there is none.
    pub fn load(path: &std::path::Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn log_max_bytes(&self) -> u64 {
        self.log_max_mb.saturating_mul(1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_are_the_defaults() {
        let c = Config::parse("concurrency = 2\n").unwrap();
        assert_eq!(c, Config { concurrency: 2, ..Config::default() });
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn a_typo_is_an_error_not_a_default() {
        assert!(Config::parse("concurency = 2\n").is_err());
        assert!(Config::parse("concurrency = 0\n").is_err());
    }
}
