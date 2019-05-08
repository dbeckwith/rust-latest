use chrono::{Duration, NaiveDate};
use clap::arg_enum;
use failure::{bail, Error, ResultExt};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::{collections::HashMap, io::Read};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(
    about = "Determines the last known complete build of a Rust toolchain."
)]
struct Config {
    #[structopt(
        short = "c",
        long = "channel",
        help = "Release channel to use.",
        default_value = "stable"
    )]
    channel: String,

    #[structopt(
        short = "a",
        long = "max-age",
        help = "Number of days back to search for viable builds. This is \
                relative to the latest release of the channel.",
        default_value = "90"
    )]
    max_age: usize,

    #[structopt(
        short = "t",
        long = "targets",
        help = "Which set of targets to filter by, either all Tier-1 targets \
                or only the current target.",
        default_value = "all",
        raw(possible_values = "&[\"all\", \"current\"]"),
        case_insensitive = true
    )]
    targets: TargetsOpt,
}

arg_enum! {
    #[derive(Debug)]
    enum TargetsOpt {
        All,
        Current,
    }
}

const CURRENT_TARGET: &str = env!("TARGET");

/// All Rust Tier 1 targets.
/// https://github.com/rust-lang/rustup-components-history/blob/0541afebe68f77acf13cdab4af9ee7db69183660/html/src/opts.rs#L93
static TIER_1_TARGETS: &[&str] = &[
    "i686-apple-darwin",
    "i686-pc-windows-gnu",
    "i686-pc-windows-msvc",
    "i686-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-gnu",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
];

// TODO: don't ignore if on the targets where they appear
static IGNORED_PACKAGES: &[&str] = &["lldb-preview", "rust-mingw"];

/// A rustup manifest.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
    /// A date for which the manifest is generated.
    pub date: NaiveDate,
    /// A map of available packages and their targets.
    #[serde(rename = "pkg")]
    pub packages: HashMap<String, PackageTargets>,
}

/// Package info.
#[derive(Debug, Deserialize)]
pub struct PackageTargets {
    /// Maps targets onto package availability info.
    #[serde(rename = "target")]
    pub targets: HashMap<String, PackageInfo>,
}

/// A per-target package information.
#[derive(Debug, Deserialize)]
pub struct PackageInfo {
    /// If a package is available for a specific target.
    pub available: bool,
}

const BASE_URL: &str = "https://static.rust-lang.org/dist";

fn get_manifest(client: &Client, url: &str) -> Result<Option<Manifest>, Error> {
    let mut res = client.get(url).send().context("error making request")?;
    match res.status() {
        StatusCode::OK => {},
        StatusCode::NOT_FOUND => {
            return Ok(None);
        },
        code => bail!("error getting latest manifest from {}: {}", url, code),
    }
    let mut content =
        Vec::with_capacity(res.content_length().unwrap_or(0) as usize);
    res.read_to_end(&mut content)
        .context("error downloading latest manifest")?;
    let manifest =
        toml::from_slice(&content).context("error reading latest manifest")?;
    Ok(Some(manifest))
}

fn filter_manifest(
    manifest: &Manifest,
    ignored_packages: &[&str],
    targets: &[&str],
) -> bool {
    return manifest
        .packages
        .iter()
        .filter(|(package, _)| !ignored_packages.contains(&package.as_str()))
        .flat_map(|(_, package_targets)| {
            targets
                .iter()
                .filter_map(|&target| package_targets.targets.get(target))
                .collect::<Vec<_>>()
        })
        .all(|package_info| package_info.available);
}

fn find_latest_viable_date(
    channel: &str,
    max_age: usize,
    ignored_packages: &[&str],
    targets: &[&str],
) -> Result<Option<NaiveDate>, Error> {
    let client = Client::new();

    let latest_manifest = match get_manifest(
        &client,
        &format!("{}/channel-rust-{}.toml", BASE_URL, channel),
    )? {
        Some(manifest) => manifest,
        None => bail!("no manifest found for release channel {}", channel),
    };
    let start_date = latest_manifest.date;

    let dates = (1..max_age).filter_map(|day| {
        start_date.checked_sub_signed(Duration::days(day as i64))
    });
    let manifests =
        std::iter::once(Ok(Some(latest_manifest))).chain(dates.map(|date| {
            get_manifest(
                &client,
                &format!("{}/{}/channel-rust-{}.toml", BASE_URL, date, channel),
            )
        }));

    for manifest in manifests {
        if let Some(manifest) = manifest? {
            if filter_manifest(&manifest, ignored_packages, targets) {
                return Ok(Some(manifest.date));
            }
        }
    }
    Ok(None)
}

fn run() -> Result<(), Error> {
    let config = Config::from_args();

    if let Some(date) = find_latest_viable_date(
        &config.channel,
        config.max_age,
        IGNORED_PACKAGES,
        match config.targets {
            TargetsOpt::All => TIER_1_TARGETS,
            TargetsOpt::Current => &[CURRENT_TARGET],
        },
    )? {
        println!("{}-{}", config.channel, date);
    } else {
        bail!("no viable {} build found", config.channel);
    }

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{}", error);
        for cause in error.iter_causes() {
            eprintln!("\tcaused by: {}", cause)
        }
        if std::env::var("RUST_BACKTRACE") == Ok("1".to_string()) {
            eprintln!("{}", error.backtrace());
        }
        std::process::exit(1);
    }
}
