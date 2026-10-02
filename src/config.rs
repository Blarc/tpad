use std::{env, net::SocketAddr, path::PathBuf};

use crate::{auth::AuthConfig, error::AppError};

pub const DEFAULT_MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    pub listen_addr: SocketAddr,
    pub max_file_bytes: usize,
    pub auth: Option<AuthConfig>,
}

impl Config {
    pub fn from_env() -> Result<Self, AppError> {
        let data_dir = env::var_os("TPAD_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/data"));

        let listen_addr = env::var("TPAD_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
            .parse()
            .map_err(|_| AppError::Config("TPAD_LISTEN_ADDR must be a valid socket address"))?;

        let max_file_bytes = match env::var("TPAD_MAX_FILE_BYTES") {
            Ok(value) => value
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or(AppError::Config(
                    "TPAD_MAX_FILE_BYTES must be a positive integer",
                ))?,
            Err(env::VarError::NotPresent) => DEFAULT_MAX_FILE_BYTES,
            Err(env::VarError::NotUnicode(_)) => {
                return Err(AppError::Config("TPAD_MAX_FILE_BYTES must be valid UTF-8"));
            }
        };

        let username = env::var("TPAD_AUTH_USERNAME").ok();
        let password = env::var("TPAD_AUTH_PASSWORD").ok();
        let auth = match (username, password) {
            (None, None) => None,
            (Some(username), Some(password)) if username.is_empty() && password.is_empty() => None,
            (Some(username), Some(password)) if !username.is_empty() && !password.is_empty() => {
                Some(AuthConfig::new(&username, &password))
            }
            _ => {
                return Err(AppError::Config(
                    "TPAD_AUTH_USERNAME and TPAD_AUTH_PASSWORD must both be set and non-empty",
                ));
            }
        };

        Ok(Self {
            data_dir,
            listen_addr,
            max_file_bytes,
            auth,
        })
    }
}
