---
layout: default
title: Installation
---

# Installation

## Pre-built Binaries

Download the latest release binary from GitHub Releases:

```bash
curl -sSL https://github.com/pbsladek/timebomb/releases/latest/download/timebomb-linux-x86_64 \
  -o /usr/local/bin/timebomb
chmod +x /usr/local/bin/timebomb
```

Available release assets include Linux, macOS, and Windows builds.

## Cargo

```bash
cargo install timebomb-cli --locked
```

The package name is `timebomb-cli`; the installed executable is `timebomb`.

## Docker

```bash
docker run --rm -v "$PWD:/work" -w /work pwbsladek/timebomb:latest sweep .
```

The Docker image is built from Docker Hardened Images with a Rust builder and a
distroless runtime image.

## Source

```bash
git clone https://github.com/pbsladek/timebomb
cd timebomb
cargo install --path . --locked
```

## Shell Completions

```bash
timebomb completions bash
timebomb completions zsh
timebomb completions fish
```

For zsh:

```bash
timebomb completions zsh > ~/.zsh/completions/_timebomb
```

