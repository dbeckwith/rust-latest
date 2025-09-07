use anyhow::{Context, Result, bail};
use chrono::{Duration, NaiveDate};
use clap::{Parser, ValueEnum};
use maplit::hashset;
use regex::Regex;
use reqwest::{StatusCode, blocking::Client};
use serde::Deserialize;
use std::{collections::HashMap, io::Read};

#[derive(Debug, Parser)]
#[clap(
    about = "Determines the last known complete build of a Rust toolchain.",
    rename_all = "kebab"
)]
struct Config {
    #[clap(
        short = 'c',
        long,
        help = "Release channel to use.",
        default_value = "stable"
    )]
    channel: String,

    #[clap(
        short = 'p',
        long,
        help = "Which package profile to use.",
        value_enum,
        default_value = "default"
    )]
    profile: ProfileOpt,

    #[clap(
        short = 'a',
        long,
        help = "Number of days back to search for viable builds. This is \
                relative to the latest release of the channel.",
        default_value = "90"
    )]
    max_age: usize,

    #[clap(
        short = 't',
        long,
        help = "Which set of targets to filter by, either all Tier-1 targets \
                or only the current target.",
        value_enum,
        default_value = "all"
    )]
    targets: TargetsOpt,

    #[clap(
        short = 'd',
        long,
        help = "Whether date-stamped toolchains like stable-2019-04-25 should \
                be used instead of version numbers for stable releases."
    )]
    force_date: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ProfileOpt {
    Complete,
    Default,
    Minimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TargetsOpt {
    All,
    Current,
}

const CURRENT_TARGET: &str = env!("TARGET");

/// All Rust [Tier 1 targets](https://doc.rust-lang.org/nightly/rustc/platform-support.html).
static TIER_1_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "aarch64-pc-windows-msvc",
    "aarch64-unknown-linux-gnu",
    "i686-pc-windows-msvc",
    "i686-unknown-linux-gnu",
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

    let Some(latest_manifest) = get_manifest(
        &client,
        &format!("{}/channel-rust-{}.toml", BASE_URL, channel),
    )?
    else {
        bail!("no manifest found for release channel {}", channel)
    };

    let start_date = latest_manifest.date;
    let dates = (1..max_age).filter_map(|day| {
        start_date.checked_sub_signed(Duration::days(day as i64))
    });

    std::iter::once(Ok(latest_manifest))
        .chain(dates.filter_map(|date| {
            get_manifest(
                &client,
                &format!("{}/{}/channel-rust-{}.toml", BASE_URL, date, channel),
            )
            .transpose()
        }))
        .find(|manifest| {
            manifest.as_ref().map_or(true, |manifest| {
                let profile = manifest.profiles[match profile {
                    ProfileOpt::Complete => "complete",
                    ProfileOpt::Default => "default",
                    ProfileOpt::Minimal => "minimal",
                }]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
                filter_manifest(manifest, &profile, ignored_packages, targets)
            })
        })
        .transpose()
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
    if !force_date
        && channel == "stable"
        && let Some(version) = get_rust_version(manifest)
    {
        return version;
    }

    format!("{}-{}", channel, manifest.date)
}

fn run() -> Result<()> {
    let config = Config::parse();

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

    let Some(manifest) = find_latest_viable_manifest(
        &config.channel,
        config.profile,
        config.max_age,
        &ignored_packages,
        match config.targets {
            TargetsOpt::All => TIER_1_TARGETS,
            TargetsOpt::Current => &[CURRENT_TARGET],
        },
    )?
    else {
        bail!("no viable {} build found", config.channel)
    };

    let toolchain_name =
        make_toolchain_name(&manifest, &config.channel, config.force_date);
    println!("{}", toolchain_name);

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
