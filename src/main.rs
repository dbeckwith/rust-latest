use anyhow::{bail, Context, Result};
use chrono::{Duration, NaiveDate};
use clap::arg_enum;
use maplit::hashset;
use regex::Regex;
use reqwest::{blocking::Client, StatusCode};
use serde::Deserialize;
use std::{collections::HashMap, io::Read};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(
    about = "Determines the last known complete build of a Rust toolchain.",
    rename_all = "kebab"
)]
struct Config {
    #[structopt(
        short = "c",
        help = "Release channel to use.",
        default_value = "stable"
    )]
    channel: String,

    #[structopt(
        short = "p",
        help = "Which package profile to use.",
        default_value = "default",
        possible_values = &["complete", "default", "minimal"],
        case_insensitive = true
    )]
    profile: ProfileOpt,

    #[structopt(
        short = "a",
        help = "Number of days back to search for viable builds. This is \
                relative to the latest release of the channel.",
        default_value = "90"
    )]
    max_age: usize,

    #[structopt(
        short = "t",
        help = "Which set of targets to filter by, either all Tier-1 targets \
                or only the current target.",
        default_value = "all",
        possible_values = &["all", "current"],
        case_insensitive = true
    )]
    targets: TargetsOpt,

    #[structopt(
        short = "d",
        help = "Whether date-stamped toolchains like stable-2019-04-25 should \
                be used instead of version numbers for stable releases."
    )]
    force_date: bool,
}

arg_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ProfileOpt {
        Complete,
        Default,
        Minimal,
    }
}

arg_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Manifest {
    date: NaiveDate,
    #[serde(rename = "pkg")]
    packages: HashMap<String, PackageTargets>,
    profiles: HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct PackageTargets {
    version: String,
    #[serde(rename = "target")]
    targets: HashMap<String, PackageInfo>,
}

#[derive(Debug, Deserialize)]
struct PackageInfo {
    available: bool,
}

const BASE_URL: &str = "https://static.rust-lang.org/dist";

// TODO: use async
fn get_manifest(client: &Client, url: &str) -> Result<Option<Manifest>> {
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
    profile: &[&str],
    ignored_packages: &[&str],
    targets: &[&str],
) -> bool {
    manifest
        .packages
        .iter()
        .filter(|(package, _package_targets)| {
            let package = package.as_str();
            if ignored_packages.contains(&package) {
                return false;
            }
            if !profile.contains(&package) {
                return false;
            }
            true
        })
        .flat_map(|(_package, package_targets)| {
            targets
                .iter()
                .filter_map(|&target| package_targets.targets.get(target))
                .collect::<Vec<_>>()
        })
        .all(|package_info| package_info.available)
}

fn find_latest_viable_manifest(
    channel: &str,
    profile: ProfileOpt,
    max_age: usize,
    ignored_packages: &[&str],
    targets: &[&str],
) -> Result<Option<Manifest>> {
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
            let profile = manifest.profiles[match profile {
                ProfileOpt::Complete => "complete",
                ProfileOpt::Default => "default",
                ProfileOpt::Minimal => "minimal",
            }]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
            if filter_manifest(&manifest, &profile, ignored_packages, targets) {
                return Ok(Some(manifest));
            }
        }
    }

    Ok(None)
}

fn get_rust_version(manifest: &Manifest) -> Option<String> {
    let package = manifest.packages.get("rust")?;
    let captures = Regex::new(r#"^(\d+\.\d+\.\d+)"#)
        .unwrap()
        .captures(&package.version)?;
    let version = &captures[1];
    Some(version.to_string())
}

fn make_toolchain_name(
    manifest: &Manifest,
    channel: &str,
    force_date: bool,
) -> String {
    if !force_date && channel == "stable" {
        if let Some(version) = get_rust_version(manifest) {
            return version;
        }
    }

    format!("{}-{}", channel, manifest.date)
}

fn run() -> Result<()> {
    let config = Config::from_args();

    let mut ignored_packages = hashset! {
        "lldb-preview",
        "rust-mingw",
    };
    if config.targets == TargetsOpt::Current {
        let allowed_packages = match CURRENT_TARGET {
            "i686-apple-darwin" | "x86_64-apple-darwin" => {
                hashset! {
                    "lldb-preview",
                }
            },
            "i686-pc-windows-gnu" | "x86_64-pc-windows-gnu" => {
                hashset! {
                    "rust-mingw",
                }
            },
            _ => Default::default(),
        };
        ignored_packages = &ignored_packages - &allowed_packages;
    }
    let ignored_packages = ignored_packages.into_iter().collect::<Vec<_>>();

    if let Some(manifest) = find_latest_viable_manifest(
        &config.channel,
        config.profile,
        config.max_age,
        &ignored_packages,
        match config.targets {
            TargetsOpt::All => TIER_1_TARGETS,
            TargetsOpt::Current => &[CURRENT_TARGET],
        },
    )? {
        let toolchain_name =
            make_toolchain_name(&manifest, &config.channel, config.force_date);
        println!("{}", toolchain_name);
    } else {
        bail!("no viable {} build found", config.channel);
    }

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{}", error);
        for cause in error.chain().skip(1) {
            eprintln!("\tcaused by: {}", cause)
        }
        std::process::exit(1);
    }
}
