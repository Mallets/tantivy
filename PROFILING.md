# Indexing Profiling Guide

## Overview

This guide explains how to profile tantivy's indexing pipeline using CPU flamegraphs, heap allocation tracking, and the existing benchmark suite.

## Prerequisites

```bash
cargo install flamegraph
cargo install samply
```

On macOS, `cargo flamegraph` requires the full Xcode installation (not just Command Line Tools):

```bash
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
```

## Profiling Binary

`benches/profile_indexing.rs` is a standalone binary that indexes a JSON-lines dataset in a loop. It uses a single indexer thread with no merging to isolate pure indexing cost.

```
Usage: profile_indexing [OPTIONS]

Options:
  -i, --iterations <N>       Number of indexing iterations to run [default: 1]
  -f, --file <PATH>          Path to a JSON-lines input file (omit for embedded wiki.json)
  -b, --buffer-size <BYTES>  Writer heap budget in bytes [default: 50000000]
```

## CPU Flamegraph

Generate an SVG flamegraph with `cargo flamegraph`:

```bash
# Small embedded dataset (quick iteration)
CARGO_TARGET_DIR=target cargo flamegraph --profile profiling --bin profile_indexing -o flamegraph.svg -- -i 5

# Large external dataset (e.g. full Wikipedia)
CARGO_TARGET_DIR=target cargo flamegraph --profile profiling --bin profile_indexing -o flamegraph.svg -- -f ~/data/wiki-articles.json
```

Open `flamegraph.svg` in a browser to explore the interactive SVG.

### Alternative: samply (interactive Firefox Profiler)

```bash
CARGO_TARGET_DIR=target cargo build --profile profiling --bin profile_indexing
samply record ./target/profiling/profile_indexing -- -i 5
```

This opens the Firefox Profiler UI in your browser for interactive exploration.

## Heap Allocation Profiling (dhat)

Build and run with the `dhat-heap` feature to capture heap allocation data:

```bash
CARGO_TARGET_DIR=target cargo run --profile profiling --features dhat-heap --bin profile_indexing
```

Output: `dhat-heap.json` in the repo root. View it at https://nnethercote.github.io/dh_view/dh_view.html.

Key metrics from the profile:
- **Total**: bytes allocated over the program's lifetime
- **At t-gmax**: peak memory usage
- **At t-end**: bytes still allocated at exit (potential leaks)

## Baseline Benchmarks

Run the existing Criterion benchmarks for indexing and tokenization:

```bash
# Indexing benchmarks (HDFS, GitHub, Wikipedia datasets)
CARGO_TARGET_DIR=target cargo bench --bench index-bench

# Tokenizer benchmarks (default vs dynamic analyzer on alice.txt)
CARGO_TARGET_DIR=target cargo bench --bench analyzer
```

### Latest Baseline Results

**Indexing (index-bench):**

| Benchmark | Time | Throughput |
|---|---|---|
| index-hdfs / only-indexed-no-commit | 155.7 ms | 137.3 MiB/s |
| index-hdfs / only-indexed-with-commit | 235.7 ms | 90.7 MiB/s |
| index-hdfs / only-fast-no-commit | 32.1 ms | 666.1 MiB/s |
| index-hdfs / only-fast-with-commit | 86.0 ms | 248.4 MiB/s |
| index-hdfs / dynamic-no-commit | 210.7 ms | 101.4 MiB/s |
| index-hdfs / dynamic-with-commit | 327.0 ms | 65.4 MiB/s |
| index-gh / no-commit | 22.2 ms | 102.0 MiB/s |
| index-gh / fast | 4.4 ms | 519.5 MiB/s |
| index-gh / fast-with-commit | 13.7 ms | 165.4 MiB/s |
| index-wiki / no-commit | 11.3 ms | 97.7 MiB/s |
| index-wiki / with-commit | 29.6 ms | 37.3 MiB/s |

**Tokenization (analyzer):**

| Benchmark | Time |
|---|---|
| default-tokenize-alice | 631 us |
| dynamic-tokenize-alice | 865 us |

## tantivy-cli Wikipedia Indexing

For profiling with the full Wikipedia dataset through the CLI pipeline:

```bash
cd ~/repos/mallets-tantivy-cli

# Download dataset (2.34 GB compressed)
# wget https://www.dropbox.com/s/wwnfnu441w1ec9p/wiki-articles.json.bz2
# bunzip2 wiki-articles.json.bz2

# Create and set up the index
mkdir -p wikipedia-index
CARGO_TARGET_DIR=target cargo run --profile profiling -- new -i ./wikipedia-index

# Flamegraph the indexing (single thread, no merge)
CARGO_TARGET_DIR=target cargo flamegraph --profile profiling --bin tantivy -o flamegraph-wiki.svg \
  -- index -i ./wikipedia-index -f ~/data/wiki-articles.json -t 1 --nomerge
```

## Cargo Configuration

The `[profile.profiling]` section in `Cargo.toml` (both tantivy and tantivy-cli) provides optimized builds with debug symbols:

```toml
[profile.profiling]
inherits = "release"
debug = 2
strip = false
```

The `dhat-heap` feature gates the dhat allocator so it has zero cost when not enabled:

```toml
[features]
dhat-heap = ["dep:dhat"]
```

## Output Files

| File | Description |
|---|---|
| `flamegraph.svg` | Interactive CPU flamegraph (open in browser) |
| `dhat-heap.json` | Heap allocation profile (view at dh_view.html) |
