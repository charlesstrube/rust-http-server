use std::path::PathBuf;

#[derive(Default)]
pub struct Config {
    directory: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("--directory requires a value")]
    MissingDirectory,
}

impl Config {
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Result<Self, ConfigError> {
        let mut args_loops = args.into_iter();

        while let Some(arg) = args_loops.next() {
            if arg == "--directory" {
                let value = args_loops.next();

                let Some(path) = value else {
                    return Err(ConfigError::MissingDirectory);
                };

                let directory = Some(PathBuf::from(path));
                return Ok(Self { directory });
            }
        }

        Ok(Self { directory: None })
    }

    pub fn directory(&self) -> Option<&std::path::Path> {
        self.directory.as_deref()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::Path;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args_means_no_directory() {
        assert_eq!(Config::from_args(args(&[])).unwrap().directory(), None);
    }

    #[test]
    fn default_has_no_directory() {
        assert_eq!(Config::default().directory(), None);
    }

    #[test]
    fn reads_directory_flag() {
        let config = Config::from_args(args(&["--directory", "/tmp/"])).unwrap();

        assert_eq!(config.directory(), Some(Path::new("/tmp/")));
    }

    #[test]
    fn directory_without_trailing_slash() {
        let config = Config::from_args(args(&["--directory", "/var/data"])).unwrap();

        assert_eq!(config.directory(), Some(Path::new("/var/data")));
    }

    #[test]
    fn directory_flag_without_value_is_an_error() {
        assert!(matches!(
            Config::from_args(args(&["--directory"])),
            Err(ConfigError::MissingDirectory)
        ));
    }
}
