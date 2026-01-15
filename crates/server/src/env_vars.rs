use std::error::Error;
use std::str::FromStr;

use anyhow::{Context, anyhow};

/// Reads an environment variable for the current process.
///
/// Compared to [std::env::var] there are a couple of differences:
///
/// - [var] uses [dotenvy] which loads the `.env` file from the current or parent directories before
///   returning the value.
///
/// - [var] returns `Ok(None)` (instead of `Err`) if an environment variable wasn't set.
#[track_caller]
pub fn var(key: &str) -> anyhow::Result<Option<String>> {
    match dotenvy::var(key) {
        Ok(content) => Ok(Some(content)),
        Err(dotenvy::Error::EnvVar(std::env::VarError::NotPresent)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Reads an environment variable for the current process, and fails if it was
/// not found.
///
/// Compared to [std::env::var] there are a couple of differences:
///
/// - [var] uses [dotenvy] which loads the `.env` file from the current or parent directories before
///   returning the value.
#[track_caller]
pub fn required_var(key: &str) -> anyhow::Result<String> {
    required(var(key), key)
}

/// Reads an environment variable for the current process, and parses it if
/// it is set.
///
/// Compared to [std::env::var] there are a couple of differences:
///
/// - [var] uses [dotenvy] which loads the `.env` file from the current or parent directories before
///   returning the value.
///
/// - [var] returns `Ok(None)` (instead of `Err`) if an environment variable wasn't set.
#[track_caller]
pub fn var_parsed<R>(key: &str) -> anyhow::Result<Option<R>>
where
    R: FromStr,
    R::Err: Error + Send + Sync + 'static,
{
    match var(key) {
        Ok(Some(content)) => {
            Ok(Some(content.parse().with_context(|| {
                format!("Failed to parse {key} environment variable")
            })?))
        }
        Ok(None) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Reads an environment variable for the current process, and parses it if
/// it is set or fails otherwise.
///
/// Compared to [std::env::var] there are a couple of differences:
///
/// - [var] uses [dotenvy] which loads the `.env` file from the current or parent directories before
///   returning the value.
#[track_caller]
pub fn required_var_parsed<R>(key: &str) -> anyhow::Result<R>
where
    R: FromStr,
    R::Err: Error + Send + Sync + 'static,
{
    required(var_parsed(key), key)
}

fn required<T>(res: anyhow::Result<Option<T>>, key: &str) -> anyhow::Result<T> {
    match res {
        Ok(opt) => opt.ok_or_else(|| anyhow!("Failed to find required {key} environment variable")),
        Err(error) => Err(error),
    }
}

/// Reads an environment variable and parses it as a comma-separated list, or
/// returns an empty list if the variable is not set.
#[track_caller]
pub fn list(key: &str) -> anyhow::Result<Vec<String>> {
    let values = match var(key)? {
        None => vec![],
        Some(s) if s.is_empty() => vec![],
        Some(s) => s.split(',').map(str::trim).map(String::from).collect(),
    };

    Ok(values)
}

/// Reads an environment variable and parses it as a comma-separated list, or
/// returns an empty list if the variable is not set. Each individual value is
/// parsed using [FromStr].
#[track_caller]
pub fn list_parsed<T, E, F, C>(key: &str, f: F) -> anyhow::Result<Vec<T>>
where
    F: Fn(&str) -> C,
    C: Context<T, E>,
{
    let values = match var(key)? {
        None => vec![],
        Some(s) if s.is_empty() => vec![],
        Some(s) => s
            .split(',')
            .map(str::trim)
            .map(|s| {
                f(s).with_context(|| {
                    format!("Failed to parse value \"{s}\" of {key} environment variable")
                })
            })
            .collect::<Result<_, _>>()?,
    };

    Ok(values)
}
