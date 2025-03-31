# starknet-scrape

This is an experimental exploratory tool for downloading and parsing [Starknet](https://www.starknet.io/) state updates from [Ethereum](https://ethereum.org/en/) mainnet (where Starknet, as an L2, publishes them).

Currently, only the newer (since March 2024) blob-based state update formats are supported. Subject to that restriction, the tool is aiming to implement (and validate) the Starknet state update [specification](https://docs.starknet.io/architecture-and-concepts/network-architecture/data-availability/).

This project reuses both code and ideas from [majin-blob](https://github.com/AbdelStark/majin-blob), Starkware [Sequencer](https://github.com/starkware-libs/sequencer) and EQ Labs [zksync-state-reconstruct](https://github.com/eqlabs/zksync-state-reconstruct).

## Usage

This is a [Rust](https://www.rust-lang.org/) project and doesn't have binary releases, so it requires a recent [Rust toolchain](https://rustup.rs/) to compile. Compilation can be done the normal way with `cargo build --release`; using the release build is recommended, as parts of the blob processing pipeline (specifically the inverse Fourier transform) can be computation-heavy.

Running the project without commandline-arguments doesn't process any blobs, however - it just downloads (and discards) the first one:
```
$ cargo run
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.15s
     Running `target/debug/starknet-scrape`
2025-03-28T13:32:56.082037Z  INFO got 1 log(s)
```

Implemented command-line arguments are documented:
```
$ target/release/starknet-scrape -h
Usage: starknet-scrape [OPTIONS]

Options:
      --config-file <config.toml>  Path to config file (must exist) [default: config.toml]
  -f, --from-block <n>             First block to search for [default: 19427723]
  -c, --block-count <n>            Number of blocks to check in one `eth_getLogs` call [default: 1]
  -p, --parse                      Actually process downloaded blobs
  -d, --dump                       Dump results of various state update processing stages into the cache directory
  -l, --parse-local                Before connecting to Ethereum, scan the cache directory for previously-dumped unparsed state updates and parse them
  -a, --annotate-only              Instead of connecting to Ethereum, scan the cache directory for previously-dumped uncompressed state updates and parse them, also annotating the data with the parser's interpretation
  -0, --no-connect                 Do not connect to Ethereum
  -1, --single-shot                Call `eth_getLogs` just once, even if it does return data
  -s, --save                       Save downloaded blobs into the cache directory before processing them
  -j, --json                       Convert parsed blobs to JSON and save it into the cache directory
  -u, --prune                      When saving / dumping data, remove files for already fully-processed updates
  -h, --help                       Print help (see more with '--help')
  -V, --version                    Print version
```

The configuration file is included in the repo; it contains default URLs of servers providing Ethereum transactions and (separately) blobs and also a path to the cache directory, where the tool stores the results of various steps to help with debugging - for example, to debug parsing of some specific state update, it's possible to download it in one run, then use the downloaded (and Fourier-transformed, concatenated, optionally uncompressed and potentially even hand-edited) data as input to the parsing step in subsequent runs.
