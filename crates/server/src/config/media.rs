use regex::RegexSet;
use serde::Deserialize;

use crate::core::serde::default_true;
use crate::macros::config_example;

#[config_example(filename = "palpo-example.toml", section = "media")]
#[derive(Clone, Debug, Deserialize)]
pub struct MediaConfig {
    /// Enable the legacy unauthenticated Matrix media repository endpoints.
    /// These endpoints consist of:
    /// - /_matrix/media/*/config
    /// - /_matrix/media/*/upload
    /// - /_matrix/media/*/preview_url
    /// - /_matrix/media/*/download/*
    /// - /_matrix/media/*/thumbnail/*
    ///
    /// The authenticated equivalent endpoints are always enabled.
    ///
    /// Defaults to true for now, but this is highly subject to change, likely
    /// in the next release.
    #[serde(default = "default_true")]
    pub allow_legacy: bool,

    #[serde(default = "default_true")]
    pub freeze_legacy: bool,

    /// Check consistency of the media directory at startup:
    /// 1. When `media_compat_file_link` is enabled, this check will upgrade media when switching
    ///    back and forth between Conduit and palpo. Both options must be enabled to handle this.
    /// 2. When media is deleted from the directory, this check will also delete its database
    ///    entry.
    ///
    /// If none of these checks apply to your use cases, and your media
    /// directory is significantly large setting this to false may reduce
    /// startup time.
    #[serde(default = "default_true")]
    pub startup_check: bool,

    /// Enable backward-compatibility with Conduit's media directory by creating
    /// symlinks of media.
    ///
    /// This option is only necessary if you plan on using Conduit again.
    /// Otherwise setting this to false reduces filesystem clutter and overhead
    /// for managing these symlinks in the directory. This is now disabled by
    /// default. You may still return to upstream Conduit but you have to run
    /// palpo at least once with this set to true and allow the
    /// media_startup_check to take place before shutting down to return to
    /// Conduit.
    #[serde(default)]
    pub compat_file_link: bool,

    /// Prune missing media from the database as part of the media startup
    /// checks.
    ///
    /// This means if you delete files from the media directory the
    /// corresponding entries will be removed from the database. This is
    /// disabled by default because if the media directory is accidentally moved
    /// or inaccessible, the metadata entries in the database will be lost with
    /// sadness.
    #[serde(default)]
    pub prune_missing: bool,

    /// Vector list of regex patterns of server names that palpo will refuse
    /// to download remote media from.
    ///
    /// example: ["badserver\.tld$", "badphrase", "19dollarfortnitecards"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub prevent_downloads_from: RegexSet,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            allow_legacy: true,
            freeze_legacy: true,
            startup_check: true,
            compat_file_link: false,
            prune_missing: false,
            prevent_downloads_from: Default::default(),
        }
    }
}
