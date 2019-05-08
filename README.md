# `rust-latest`

A CLI tool to determine the latest version of the Rust toolchain. For `nightly`, it finds the latest version to include builds of all the builtin components (`clippy`, `rustfmt`, `rls`, etc.).

```console
$ rust-latest
1.34.1
$ rust-latest -c nightly
nightly-2019-05-04
```

This tool *does not* actually download the toolchain; that job is still left to `rustup`. This tool is intended to be used to set your project's `rust-toolchain` file. Read more about what this is below.

```console
$ rustup show active-toolchain
stable-x86_64-unknown-linux-gnu (default)
$ rust-latest -c nightly > rust-toolchain
$ rustup show active-toolchain
nightly-2019-05-04-x86_64-unknown-linux-gnu (overridden by '/path/to/project/rust-toolchain')
```

## The Problem

The Rust language has a lot of updates, which can be both a good thing and a bad thing. It's a good thing because new features are being added to the language constantly. It's a bad thing because (especially on `nightly`), these frequent changes can break your builds without your code changing at all!

Thankfully, `rustup` allows you to pin the Rust version for your project with the [`rust-toolchain` file](https://github.com/rust-lang/rustup.rs#the-toolchain-file). This enables a few things:

First, it prevents the version of Rust your project builds with from changing when a new version is installed on your system. This allows you to control exactly when you want to upgrade the version of Rust you're using for that project. It enables a sort of "virtual environment" for your project which is independent of the system-wide Rust toolchain (`rustup default`).

Second, it lets you declare a version of Rust that your project is known to be compatible with, much like `Cargo.toml` does for your dependencies. This is a bit different from `Cargo.toml` because you use a specific pinned version instead of a Semver range, but it serves a similar purpose.

Third, it ensures *repeatable builds* of your project: everyone who builds your project, whether it be you, another contributor, you a month from now, or your CI server, will build it with exactly the same (or a compatible set of) dependencies and toolchain.

"`rust-toolchain` sounds great!" you say, "but what do I put in it?" That's the central problem this tool aims to solve.

## The Solution

The first step to deciding what you should put in your `rust-toolchain` file is to decide what release channel you want to use. This decision is outside the scope of this tool, but you can read up on the differences between `stable`, `beta`, and `nightly` [here](https://github.com/rust-lang/rustup.rs#keeping-rust-up-to-date) and [here](https://github.com/nox/rust-rfcs/blob/master/text/0507-release-channels.md).

The next step, which this tool solves for you, is to decide which release in that channel to use. When you don't have a particular preference for a version, or just want the latest version, how do determine the actual label?

If you're using `nightly`, you can go look at the [`rustup` components history](https://rust-lang.github.io/rustup-components-history/index.html) and pick a date from there. Usually I prefer a nightly build that has all the package components available, since I need `clippy`, `rls`, and `rust-src` for development. So I scan the table for the last date that has green boxes for all those components. But if the last release to contain the package was older than a week, it wouldn't show up on the table. Also, not every day has a `nightly` release.

If you're using `stable`, you can look at the [Rust release history](https://github.com/rust-lang/rust/blob/master/RELEASES.md) to get a release version number to use.

I've never really used `beta`, so I'm not actually sure how you would normally find a list of releases for it.

But I don't want to have to do any of that manual work! I wanted a tool that could perform all of these checks automatically, which is what `rust-latest` does. Given a release channel, a list of desired components, and a list of build targets, it finds the last release on that channel which had all of the components available for every target. To make this a bit easier on the user, it actually checks for releases containing everything *except* a few components which are platform-specific. If you run the tool on one of these platforms and specify `--target current`, it will detect this case and include those components. By default, the tool will look in the `stable` channel back at most 90 days for releases which include the components for *all* [Tier 1 targets](https://forge.rust-lang.org/platform-support.html).

## Usage

Installing the tool is as easy as `cargo install rust-latest`!

```console
$ rust-latest --help
rust-latest 1.2.1
Daniel Beckwith <djbsnx@gmail.com>
Determines the last known complete build of a Rust toolchain.

USAGE:
    rust-latest [FLAGS] [OPTIONS]

FLAGS:
    -d, --force-date    Whether date-stamped toolchains like stable-2019-04-25 should be used instead of version numbers
                        for stable releases.
    -h, --help          Prints help information
    -V, --version       Prints version information

OPTIONS:
    -c, --channel <channel>    Release channel to use. [default: stable]
    -a, --max-age <max_age>    Number of days back to search for viable builds. This is relative to the latest release
                               of the channel. [default: 90]
    -t, --targets <targets>    Which set of targets to filter by, either all Tier-1 targets or only the current target.
                               [default: all]  [possible values: all, current]
```

When you run the tool, it will output a toolchain name that can be used in your `rust-toolchain` file:

```console
$ rust-latest
1.34.1
$ rust-latest --force-date
stable-2019-04-25
$ rust-latest -c nightly
nightly-2019-05-04
```

Note that the tool requires an internet connection and can take a while to complete, as it has to download release manifests that can be serveral hundred kilobytes each. The farther back in time it has to search for a viable release, the longer it will take.

## Contributing

If you have any problems using this tool or ideas for improvement, please [create an issue](https://github.com/dbeckwith/rust-latest/issues) and I'll respond as soon as I can!
