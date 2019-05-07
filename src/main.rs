use failure::{bail, ensure, Error, ResultExt};
use lazy_static::lazy_static;
use rustup_available_packages::{AvailabilityData, Downloader};
use std::collections::HashSet;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(about = "Determines the last known complete build of rust nightly.")]
struct Config {
    #[structopt(
        short = "c",
        long = "channel",
        help = "Release channel to use",
        default_value = "nightly"
    )]
    channel: String,

    #[structopt(
        short = "a",
        long = "max-age",
        help = "Number of days back to search for viable builds",
        default_value = "14"
    )]
    max_age: usize,
}

const TARGET: &str = env!("TARGET");

lazy_static! {
    static ref IGNORED_PACKAGES: HashSet<&'static str> = {
        let mut pkgs = HashSet::new();
        if !(TARGET == "i686-apple-darwin" || TARGET == "x86_64-apple-darwin") {
            pkgs.insert("lldb-preview");
        }
        if !(TARGET == "i686-pc-windows-gnu" || TARGET == "x86_64-pc-windows-gnu") {
            pkgs.insert("rust-mingw");
        }
        pkgs
    };
}

fn run() -> Result<(), Error> {
    let config = Config::from_args();

    let downloader = Downloader::with_default_source(&config.channel);
    let manifests = downloader
        .get_last_manifests(config.max_age)
        .context("error downloading manifests")?;
    let dates = manifests
        .iter()
        .map(|manifest| manifest.date)
        .collect::<Vec<_>>();
    let mut availability = AvailabilityData::default();
    availability.add_manifests(manifests);
    ensure!(
        availability.get_available_targets().contains(TARGET),
        "no availability information for target triple \"{}\"",
        TARGET
    );
    let packages = &availability.get_available_packages() - &IGNORED_PACKAGES;
    let date = dates.into_iter().find(|&date| {
        packages
            .iter()
            .all(|package| availability.get_availability_row(TARGET, package, &[date])[0])
    });
    if let Some(date) = date {
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
