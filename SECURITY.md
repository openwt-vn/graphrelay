# Security Policy

## Reporting a vulnerability

Please do **not** open a public issue. Report privately through GitHub: [Security → Report a vulnerability](https://github.com/openwt-vn/graphrelay/security/advisories/new). Include what you found, how to reproduce it, and the affected version (`gr --version`).

We aim to acknowledge reports within a few days and will keep you updated on the fix. This is a volunteer-run project, so there is no guaranteed response time or bounty.

## Supported versions

Only the latest release receives fixes while the project is pre-1.0.

## Threat model and known limits

GraphRelay is a local developer tool. Knowing its boundaries helps you judge reports and deployments:

- **The daemon binds to `127.0.0.1` only** (default port 3030). Do not expose it to a network: the MCP and REST endpoints have **no authentication** and return source code and file paths from every registered project.
- **Your code stays local.** Indexing and search make no network calls. The only exceptions are features you turn on: the optional Jev relevance judge (sends candidate snippets to the configured endpoint) and `gr update` / the install script (download release archives from GitHub).
- **Self-update and install** download release archives from GitHub; `scripts/install.sh` verifies the published sha256 checksum.
- **Indexed content is untrusted input.** Source files are parsed with tree-sitter; crashes or excessive resource use triggered by crafted files are valid reports.
- `gr setup` edits your coding tools' config files (MCP entry and instruction hook). Use `--dry-run` to preview.
