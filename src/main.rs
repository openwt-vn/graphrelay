mod offload;
mod origin_guard;
mod registry;
mod server;

use anyhow::Context;
use axum::Router;
use clap::{Parser, Subcommand};
use dialoguer::{MultiSelect, theme::ColorfulTheme};
use registry::Registry;
use std::{io::IsTerminal, path::PathBuf, sync::Arc};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "gr",
    about = "Graph-aware code search — MCP + REST",
    version = env!("GRAPHRELAY_FULL_VERSION"),
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Register the current directory as a graphrelay project.
    Init {
        /// Project name (defaults to current folder name).
        #[arg(long)]
        name: Option<String>,
    },

    /// Start the graphrelay daemon (serves all registered projects).
    Start {
        #[arg(long, default_value = "3030")]
        port: u16,
    },

    /// Stop the running daemon.
    Stop,

    /// Restart the daemon.
    Restart {
        #[arg(long, default_value = "3030")]
        port: u16,
    },

    /// Show registered projects and their index state.
    Status,

    /// Force a full re-index of the current project (daemon must not be running).
    Index {
        /// Clear the graph DB (and any leftover pre-Plan-20 vector data) and
        /// checksums before re-indexing, forcing a genuinely from-scratch index.
        #[arg(long)]
        fresh: bool,
    },

    /// Remove the current directory from the project registry.
    Unregister,

    /// Run a retrieval quality benchmark against one or more project directories.
    ///
    /// Examples:
    ///   gr bench                               # docstring-pair MRR on current directory
    ///   gr bench /path/to/project              # benchmark a specific directory
    ///   gr bench --multi ./repos               # benchmark every subdirectory of ./repos
    ///   gr bench --repo benchmarks/repos/express.yaml   # YAML-driven: search + multi-hop + impact
    ///   gr bench --repo benchmarks/repos/fastapi.yaml --report out.md
    Bench {
        /// Project root for docstring-pair MRR benchmark (defaults to current directory).
        /// Ignored when --multi or --repo is set.
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Benchmark all project subdirectories inside this directory.
        #[arg(long, value_name = "REPOS_DIR")]
        multi: Option<PathBuf>,

        /// Run a YAML-driven benchmark (search quality + multi-hop + impact accuracy).
        /// Pass the path to a repos/*.yaml config file.
        #[arg(long, value_name = "CONFIG_YAML")]
        repo: Option<PathBuf>,

        /// Write report here (.md or .html).
        #[arg(long, value_name = "PATH")]
        report: Option<PathBuf>,
    },

    /// Plan 12 D2: run a SCIP semantic indexer and write the output to the configured scip_path.
    ///
    /// Examples:
    ///   gr scip index --lang python
    ///   gr scip index --lang typescript --output .graphrelay/index.scip
    Scip {
        #[command(subcommand)]
        scip_cmd: ScipCommand,
    },

    /// Compare retrieval quality between two registered projects.
    ///
    /// Both projects must be registered, indexed, and the daemon must be running.
    /// The command extracts docstring pairs from the source, queries both projects'
    /// search endpoints, and prints a side-by-side MRR / Recall@5 / latency report.
    /// Useful for comparing two corpora, or the same corpus under two different
    /// `[retrieval]` configs (e.g. different `beta`/`gamma` weights).
    ///
    /// Examples:
    ///   gr compare project-a project-b
    ///   gr compare project-a project-b --queries 50
    ///   gr compare project-a project-b --source /path/to/source --report out.md
    Compare {
        /// Name of the baseline project.
        #[arg(name = "PROJECT_A")]
        project_a: String,

        /// Name of the candidate project.
        #[arg(name = "PROJECT_B")]
        project_b: String,

        /// Max docstring pairs to sample (default: 30).
        #[arg(long, default_value = "30")]
        queries: usize,

        /// Source directory to extract queries from.
        /// Defaults to PROJECT_A's root (both projects must share the same source).
        #[arg(long, value_name = "DIR")]
        source: Option<PathBuf>,

        /// Write a Markdown report here.
        #[arg(long, value_name = "PATH")]
        report: Option<PathBuf>,
    },

    /// Auto-configure coding tools to use GraphRelay (MCP server + instruction hook).
    ///
    /// Detects Claude Code, Cursor, OpenCode, Amp, and Codex; injects the MCP server
    /// entry into each tool's config file and appends a "use GraphRelay first" block
    /// to CLAUDE.md / AGENTS.md / .cursorrules in the current project.
    ///
    /// When run interactively (TTY), presents a checkbox menu so you can pick exactly
    /// which tools to configure. Detected tools are pre-checked.
    ///
    /// Run this once after `gr init` and `gr start`.
    ///
    /// Examples:
    ///   gr setup                  # interactive tool picker
    ///   gr setup --all            # configure all detected tools, no prompt
    ///   gr setup --dry-run        # show what would change, write nothing
    ///   gr setup --global         # write instruction hook to global file
    ///   gr setup --tool claude    # only configure Claude Code
    Setup {
        /// Write instruction hook to the global file (~/.claude/CLAUDE.md, etc.)
        /// instead of the project-local file.
        #[arg(long)]
        global: bool,

        /// Print what would change without writing any files.
        #[arg(long)]
        dry_run: bool,

        /// Configure only the named tool (claude|cursor|opencode|amp|codex).
        /// Skips the interactive picker.
        #[arg(long, value_name = "TOOL", conflicts_with = "all")]
        tool: Option<String>,

        /// Configure all detected tools without showing the picker.
        /// Useful for scripts and CI.
        #[arg(long, conflicts_with = "tool")]
        all: bool,
    },

    /// Download and install the latest release, replacing this binary in place.
    ///
    /// Stop the daemon first if it's running (`gr stop`) — the update itself
    /// is safe to run regardless (the new binary is written next to and then swapped
    /// in for the old one), but a running daemon keeps using the old code in memory
    /// until you restart it.
    Update,

    /// Internal — started by `gr start`. Do not call directly.
    #[command(hide = true)]
    Daemon { port: u16 },
}

#[derive(Subcommand)]
enum ScipCommand {
    /// Run the appropriate SCIP indexer and write the .scip output file.
    Index {
        /// Programming language to index.
        #[arg(long, value_name = "LANG")]
        lang: String,

        /// Output path for the .scip file.
        #[arg(long, value_name = "PATH", default_value = ".graphrelay/index.scip")]
        output: PathBuf,
    },

    /// Show which SCIP indexers are available in PATH.
    Detect,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Init { name } => cmd_init(name),
        Command::Start { port }   => cmd_start(port),
        Command::Stop             => cmd_stop(),
        Command::Restart { port } => { cmd_stop().ok(); cmd_start(port) }
        Command::Status                => cmd_status(),
        Command::Index { fresh }       => cmd_index(fresh),
        Command::Unregister       => cmd_unregister(),
        Command::Bench { path, multi, repo, report } => cmd_bench(path, multi, repo, report),
        Command::Compare { project_a, project_b, queries, source, report } =>
            cmd_compare(project_a, project_b, queries, source, report),
        Command::Scip { scip_cmd } => cmd_scip(scip_cmd),
        Command::Setup { global, dry_run, tool, all } => cmd_setup(global, dry_run, tool, all),
        Command::Update => cmd_update(),
        Command::Daemon { port }  => {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(cmd_daemon(port))
        }
    }
}

// ── init ─────────────────────────────────────────────────────────────────────

fn cmd_init(name: Option<String>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let name = name.unwrap_or_else(|| {
        cwd.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string()
    });

    // Create .graphrelay/ in current dir
    let local_dir = cwd.join(".graphrelay");
    std::fs::create_dir_all(&local_dir)?;

    // Write a stub config if none exists. Retrieval is lexical (FTS5/BM25) +
    // graph expansion, always — there's no embedding model to configure
    // (Plan 20 removed vector search entirely), so the generated config has
    // nothing to say about one.
    let config_path = local_dir.join("config.toml");
    if !config_path.exists() {
        std::fs::write(
            &config_path,
            "# graphrelay project config\n\
             # [index]\n\
             # ignore = [\".git/**\", \"node_modules/**\", \"target/**\", \"dist/**\", \".venv/**\", \"vendor/**\"]\n\
             # languages = [\"rust\", \"typescript\", \"python\", \"go\"]   # subset shown; omit this line for the full 31-language default\n\
             #\n\
             # [retrieval]\n\
             # final_top_n = 5\n\
             # beta  = 0.667   # graph-centrality weight\n\
             # gamma = 0.333   # BM25 / lexical weight\n",
        )?;
    }

    // Add .graphrelay/ to .gitignore (if one exists)
    let gitignore = cwd.join(".gitignore");
    if gitignore.exists() {
        let content = std::fs::read_to_string(&gitignore).unwrap_or_default();
        if !content.lines().any(|l| l.trim() == ".graphrelay/") {
            let mut updated = content;
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(".graphrelay/\n");
            std::fs::write(&gitignore, updated)?;
            println!("  → added .graphrelay/ to .gitignore");
        }
    }

    // Register in global registry
    let mut registry = Registry::load()?;
    let already = registry.find_by_root(&cwd).map(|p| p.name.clone());
    registry.add(name.clone(), cwd.clone());
    registry.save()?;

    if let Some(old_name) = already {
        if old_name != name {
            println!("Updated project '{}' → '{}' at {:?}", old_name, name, cwd);
        } else {
            println!("Project '{}' already registered at {:?}", name, cwd);
        }
    } else {
        println!("Initialized project '{name}' at {cwd:?}");
    }

    // If the daemon is already running, restart it so it picks up the new project.
    // If it's not running, tell the user to start it.
    let daemon_running = read_pid()
        .ok()
        .flatten()
        .map(is_process_alive)
        .unwrap_or(false);

    println!();
    if daemon_running {
        println!("  Restarting daemon to load '{name}'...");
        cmd_stop().ok();
        cmd_start(3030)?;
        println!("  MCP endpoint: http://localhost:3030/{name}/mcp");
    } else {
        println!("  Next steps:");
        println!("    gr start");
    }
    println!();
    println!("  Then wire up your coding tools:");
    println!("    gr setup       # auto-detect Claude Code, Cursor, OpenCode, Amp, Codex");
    println!("    gr setup --help");

    Ok(())
}

// ── start ─────────────────────────────────────────────────────────────────────

fn cmd_start(port: u16) -> anyhow::Result<()> {
    let registry = Registry::load()?;
    if registry.projects.is_empty() {
        anyhow::bail!(
            "No projects registered.\n\
             Run `gr init` in a project directory first."
        );
    }

    if let Some(pid) = read_pid()? {
        if is_process_alive(pid) {
            println!("graphrelay daemon already running (PID {pid})");
            return Ok(());
        }
        // Stale PID file — clean it up
        let _ = std::fs::remove_file(Registry::pid_path());
    }

    let log_path = Registry::log_path();
    std::fs::create_dir_all(Registry::global_dir())?;
    let log_file = std::fs::File::create(&log_path)
        .with_context(|| format!("creating log file {:?}", log_path))?;
    let log_file2 = log_file.try_clone()?;

    let exe = std::env::current_exe().context("could not determine own path")?;
    let child = std::process::Command::new(&exe)
        .args(["daemon", &port.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(log_file)
        .stderr(log_file2)
        .spawn()
        .with_context(|| format!("spawning daemon {:?}", exe))?;

    let pid = child.id();
    write_pid(pid)?;
    std::fs::write(Registry::port_path(), port.to_string())?;

    println!("graphrelay daemon started (PID {pid})");
    println!("Logs: {log_path:?}");
    println!();
    for p in &registry.projects {
        println!(
            "  {} → http://localhost:{}/{}/mcp",
            p.name, port, p.name
        );
    }

    Ok(())
}

// ── stop ──────────────────────────────────────────────────────────────────────

fn cmd_stop() -> anyhow::Result<()> {
    let Some(pid) = read_pid()? else {
        println!("graphrelay daemon is not running (no PID file)");
        return Ok(());
    };

    if !is_process_alive(pid) {
        println!("graphrelay daemon is not running (stale PID {pid})");
        let _ = std::fs::remove_file(Registry::pid_path());
        let _ = std::fs::remove_file(Registry::port_path());
        return Ok(());
    }

    send_sigterm(pid)?;
    // Wait up to 3 s for the process to exit
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if !is_process_alive(pid) {
            break;
        }
    }

    let _ = std::fs::remove_file(Registry::pid_path());
    let _ = std::fs::remove_file(Registry::port_path());
    println!("graphrelay daemon stopped (was PID {pid})");
    Ok(())
}

// ── status ────────────────────────────────────────────────────────────────────

fn cmd_status() -> anyhow::Result<()> {
    let registry = Registry::load()?;

    let (running, port) = match read_pid()? {
        Some(pid) if is_process_alive(pid) => {
            let port = read_port().unwrap_or(3030);
            println!("Daemon: running (PID {pid}, port {port})");
            (true, port)
        }
        Some(pid) => {
            println!("Daemon: stopped (stale PID {pid})");
            (false, 3030)
        }
        None => {
            println!("Daemon: stopped");
            (false, 3030)
        }
    };

    if registry.projects.is_empty() {
        println!("Projects: none — run `gr init` to register one");
        return Ok(());
    }

    println!();
    println!("{:<20} {:<10} {:>8} {:>10}  URL", "Project", "State", "Files", "Symbols");
    println!("{}", "-".repeat(72));

    for p in &registry.projects {
        if running {
            let url = format!("http://localhost:{}/{}/health", port, p.name);
            match ureq_get(&url) {
                Ok(body) => {
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let state   = v["state"].as_str().unwrap_or("?");
                    let files   = v["total_files"].as_u64().unwrap_or(0);
                    let symbols = v["total_symbols"].as_u64().unwrap_or(0);
                    println!(
                        "{:<20} {:<10} {:>8} {:>10}  http://localhost:{}/{}/mcp",
                        p.name, state, files, symbols, port, p.name
                    );
                }
                Err(_) => {
                    println!("{:<20} unreachable", p.name);
                }
            }
        } else {
            println!("{:<20} {:<10}", p.name, "stopped");
        }
    }

    Ok(())
}

// ── index ─────────────────────────────────────────────────────────────────────

fn cmd_index(fresh: bool) -> anyhow::Result<()> {
    if let Some(pid) = read_pid()? {
        if is_process_alive(pid) {
            println!("Daemon is running (PID {pid}).");
            println!("Stop it first, then re-index:");
            println!("  gr stop && gr index && gr start");
            return Ok(());
        }
    }

    let cwd = std::env::current_dir()?;
    let registry = Registry::load()?;
    let entry = registry
        .find_by_root(&cwd)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!(
            "Current directory is not a registered project.\n\
             Run `gr init` first."
        ))?;

    if fresh {
        let root = cwd.canonicalize().unwrap_or(cwd.clone());
        let data_dir = registry::project_data_dir(&root);
        let vectors_dir = data_dir.join("vectors");
        let db_path = data_dir.join("graph.db");

        if vectors_dir.exists() {
            std::fs::remove_dir_all(&vectors_dir)
                .with_context(|| format!("removing {:?}", vectors_dir))?;
            println!("Cleared vectors at {:?}", vectors_dir);
        }
        if db_path.exists() {
            std::fs::remove_file(&db_path)
                .with_context(|| format!("removing {:?}", db_path))?;
            println!("Cleared graph DB");
        }
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            run_one_shot_index(&entry).await
        })
}

// ── unregister ────────────────────────────────────────────────────────────────

fn cmd_unregister() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let mut registry = Registry::load()?;
    if registry.find_by_root(&cwd).is_none() {
        println!("Current directory is not a registered project.");
        return Ok(());
    }
    registry.remove_by_root(&cwd);
    registry.save()?;
    println!("Removed {:?} from the project registry.", cwd);
    println!("Run `gr restart` to apply the change.");
    Ok(())
}

// ── update ────────────────────────────────────────────────────────────────────
// Mirrors scripts/install.sh's download/auth logic (private-repo asset resolution,
// redirect handling), but replaces the currently-running binary in place instead of
// installing to a fresh directory.

const UPDATE_REPO: &str = "openwt-vn/graphrelay";

fn cmd_update() -> anyhow::Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    let target = release_target()?;
    let auth_header = github_auth_header();

    println!("Current version: {current_version} ({target})");
    print!("Checking latest release... ");
    use std::io::Write;
    std::io::stdout().flush().ok();

    let latest_json = curl_get(
        &format!("https://api.github.com/repos/{UPDATE_REPO}/releases/latest"),
        auth_header.as_deref(),
        None,
    )?;
    let latest: serde_json::Value = serde_json::from_str(&latest_json)
        .context("parsing GitHub releases/latest response")?;
    let tag = latest["tag_name"].as_str().ok_or_else(|| {
        anyhow::anyhow!(
            "could not read tag_name from GitHub API response — \
             pass a GITHUB_TOKEN env var if this is a private repo"
        )
    })?;
    let latest_version = tag.trim_start_matches('v');
    println!("{latest_version}");

    if version_tuple(latest_version) <= version_tuple(current_version) {
        println!("Already up to date.");
        return Ok(());
    }

    println!("Updating {current_version} → {latest_version}...");

    let archive = format!("graphrelay-{tag}-{target}.tar.gz");
    let work_dir = tempfile::tempdir().context("creating temp dir for download")?;
    let archive_path = work_dir.path().join(&archive);

    download_release_asset(UPDATE_REPO, tag, &archive, auth_header.as_deref(), &archive_path)?;

    let status = std::process::Command::new("tar")
        .args(["-xzf"])
        .arg(&archive_path)
        .arg("-C")
        .arg(work_dir.path())
        .status()
        .context("running tar to extract downloaded release")?;
    if !status.success() {
        anyhow::bail!("tar extraction failed");
    }

    let extracted_bin = work_dir.path().join("gr");
    if !extracted_bin.exists() {
        anyhow::bail!("extracted archive did not contain a `gr` binary");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&extracted_bin, std::fs::Permissions::from_mode(0o755))?;
    }

    let current_exe = std::env::current_exe().context("could not determine own path")?;
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("current executable path has no parent directory"))?;

    // Copy into the same directory as the binary being replaced, then rename —
    // renaming within one filesystem is atomic, so a crash mid-update can't leave a
    // half-written binary at `current_exe`. The copy step (not rename directly from
    // the tempdir) is needed because the tempdir and install dir may be on different
    // filesystems, where rename() would fail with EXDEV.
    let staged = install_dir.join(".graphrelay-update-tmp");
    std::fs::copy(&extracted_bin, &staged)
        .with_context(|| format!("copying new binary into {install_dir:?}"))?;
    std::fs::rename(&staged, &current_exe)
        .with_context(|| format!("replacing {current_exe:?}"))?;

    println!("✓ Updated to {latest_version} ({current_exe:?})");

    let daemon_running = read_pid().ok().flatten().map(is_process_alive).unwrap_or(false);
    if daemon_running {
        println!("  The running daemon is still using the old binary — run `gr restart` to apply the update.");
    }

    Ok(())
}

/// Map this platform to a release target triple, matching release.yml's build matrix.
fn release_target() -> anyhow::Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        // Historically gated on `ort` (ONNX Runtime bindings) shipping no
        // prebuilt binary for this target — `ort` was removed from the
        // dependency tree entirely by Plan 20 (vector search removal), so
        // that specific blocker no longer applies. Left gated regardless:
        // this only reflects what `release.yml`'s build matrix actually
        // publishes, which is a separate, unverified decision from this
        // change — not re-litigated here.
        ("macos", "x86_64") => anyhow::bail!(
            "macOS Intel (x86_64) builds are not published. \
             See https://github.com/{UPDATE_REPO}/releases"
        ),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        (os, arch) => anyhow::bail!("unsupported platform for auto-update: {os}/{arch}"),
    }
}

/// Parse a "x.y.z" version string into a tuple for ordering comparison.
/// Missing/unparseable components default to 0, so "0.1" < "0.1.1" as expected.
fn version_tuple(v: &str) -> (u64, u64, u64) {
    let mut parts = v.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// `GITHUB_TOKEN` env var, falling back to `gh auth token` if the gh CLI is
/// authenticated. Only needed while the repo is private.
fn github_auth_header() -> Option<String> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            return Some(format!("Authorization: Bearer {token}"));
        }
    }
    let out = std::process::Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() { None } else { Some(format!("Authorization: Bearer {token}")) }
}

/// Minimal HTTP GET via curl (matches `ureq_get` below — no HTTP client dependency).
fn curl_get(url: &str, auth_header: Option<&str>, accept: Option<&str>) -> anyhow::Result<String> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-fsSL"]);
    if let Some(h) = auth_header {
        cmd.args(["-H", h]);
    }
    if let Some(a) = accept {
        cmd.args(["-H", &format!("Accept: {a}")]);
    }
    cmd.arg(url);
    let out = cmd.output().context("running curl")?;
    if !out.status.success() {
        anyhow::bail!("curl GET {url} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Download a release asset, handling both public and private repos.
///
/// For a private repo, `github.com/.../releases/download/...` 404s — the asset must
/// be fetched through the API (`releases/tags/{tag}`) to find its numeric asset id,
/// then downloaded from the assets API endpoint with `Accept: application/octet-stream`.
/// GitHub redirects that request to S3; the Authorization header must NOT be forwarded
/// on that redirect (S3 rejects it), so the redirect is resolved manually rather than
/// via curl's `-L`.
fn download_release_asset(
    repo: &str,
    tag: &str,
    archive_name: &str,
    auth_header: Option<&str>,
    dest: &std::path::Path,
) -> anyhow::Result<()> {
    let Some(auth_header) = auth_header else {
        // Public repo: plain download URL works with no auth.
        let url = format!("https://github.com/{repo}/releases/download/{tag}/{archive_name}");
        let out = std::process::Command::new("curl")
            .args(["-fsSL", &url, "-o"])
            .arg(dest)
            .output()
            .context("running curl")?;
        if !out.status.success() {
            anyhow::bail!("download failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        return Ok(());
    };

    let release_json = curl_get(
        &format!("https://api.github.com/repos/{repo}/releases/tags/{tag}"),
        Some(auth_header),
        None,
    )?;
    let release: serde_json::Value =
        serde_json::from_str(&release_json).context("parsing release JSON")?;
    let asset_url = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str() == Some(archive_name))
        .and_then(|a| a["url"].as_str())
        .ok_or_else(|| anyhow::anyhow!("asset '{archive_name}' not found in release {tag}"))?;

    // HEAD the asset API URL to resolve the S3 redirect without downloading yet.
    let head = std::process::Command::new("curl")
        .args([
            "-sI",
            "-H", auth_header,
            "-H", "Accept: application/octet-stream",
            asset_url,
        ])
        .output()
        .context("running curl HEAD")?;
    let head_text = String::from_utf8_lossy(&head.stdout);
    let redirect = head_text
        .lines()
        .find(|l| l.to_lowercase().starts_with("location:"))
        .map(|l| l.split_once(':').map(|(_, v)| v).unwrap_or("").trim().to_string());

    let out = if let Some(s3_url) = redirect {
        std::process::Command::new("curl")
            .args(["-fsSL", &s3_url, "-o"])
            .arg(dest)
            .output()
    } else {
        std::process::Command::new("curl")
            .args([
                "-fsSL",
                "-H", auth_header,
                "-H", "Accept: application/octet-stream",
                asset_url,
                "-o",
            ])
            .arg(dest)
            .output()
    }
    .context("running curl")?;

    if !out.status.success() {
        anyhow::bail!("download failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

// ── daemon (internal) ─────────────────────────────────────────────────────────

async fn cmd_daemon(port: u16) -> anyhow::Result<()> {
    // Unconditional (not gated by RUST_LOG) so `daemon.log` always identifies
    // which exact build is running — see build.rs for what's embedded.
    println!("graphrelay {}", env!("GRAPHRELAY_FULL_VERSION"));

    let registry = Registry::load()?;

    let mut app = Router::new();
    let shutdown = Arc::new(tokio::sync::Notify::new());

    for entry in &registry.projects {
        if !entry.root.exists() {
            tracing::warn!("project root {:?} not found — skipping '{}'", entry.root, entry.name);
            continue;
        }
        match server::build_project_router(entry, shutdown.clone()).await {
            Ok(router) => {
                tracing::info!("serving /{} → {:?}", entry.name, entry.root);
                app = app.nest(&format!("/{}", entry.name), router);
            }
            Err(e) => {
                tracing::error!("failed to start project '{}': {e}", entry.name);
            }
        }
    }

    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding to {addr} — is the port already in use?"))?;
    tracing::info!("graphrelay daemon listening on http://{addr}");

    let app = app.layer(axum::middleware::from_fn(origin_guard::local_only));
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Signal watcher workers to flush pending files then exit.
    shutdown.notify_waiters();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    tracing::info!("daemon shutdown complete");
    Ok(())
}

// ── one-shot index (no server) ────────────────────────────────────────────────

async fn run_one_shot_index(entry: &registry::ProjectEntry) -> anyhow::Result<()> {
    use graphrelay_core::{
        retrieval::SourceLoader,
        types::{IndexState, IndexStatus},
    };
    use graphrelay_parser::{Extractor, LanguageRegistry};
    use graphrelay_store::{GraphDb, StoreCoordinator};
    use graphrelay_watcher::{FileWatcher, IndexWorker};
    use tokio::sync::RwLock;

    let root = entry.root.canonicalize().unwrap_or_else(|_| entry.root.clone());
    let data_dir = registry::project_data_dir(&root);
    std::fs::create_dir_all(&data_dir)?;

    let db_path = data_dir.join("graph.db");

    // Same lookup as the daemon: `gr index` used to ignore `graphrelay.toml`,
    // so the next daemon start re-indexed or pruned whatever the configs disagreed on.
    let config = server::load_project_config(&root)?;

    let graph_db = Arc::new(GraphDb::open(&db_path)?);
    let coordinator = Arc::new(StoreCoordinator::new(graph_db.clone()));

    let source_loader = Arc::new(SourceLoader::new());

    if let Err(e) = graph_db.gc_orphan_fts() {
        tracing::warn!("lexical-index orphan cleanup failed: {e}");
    }

    let watch_status = Arc::new(RwLock::new(IndexStatus {
        state:            IndexState::Indexing,
        last_indexed_at:  None,
        pending_count:    0,
        total_symbols:    0,
        total_files:      0,
        last_error:       None,
        semantic_pending: 0,
    }));
    let mut combined = config.watcher.ignore.clone();
    combined.extend(config.index.ignore.iter().cloned());
    let watcher = Arc::new(FileWatcher::new(root.clone(), &combined, &config.index.languages)?);
    let extractor = Arc::new(Extractor::new(LanguageRegistry::with_defaults()).with_root(root.clone()));
    let worker = Arc::new(IndexWorker::new(
        watcher,
        extractor,
        coordinator,
        watch_status,
        source_loader,
        config.watcher,
        config.scip,
    ));

    println!("Indexing '{}'…", entry.name);
    graphrelay_watcher::full_index(&root, &worker).await?;
    let files   = graph_db.total_file_count().unwrap_or(0);
    let symbols = graph_db.total_symbol_count().unwrap_or(0);
    println!("Done: {files} files, {symbols} symbols");
    Ok(())
}

// ── bench ─────────────────────────────────────────────────────────────────────

fn cmd_bench(
    path:   PathBuf,
    multi:  Option<PathBuf>,
    repo:   Option<PathBuf>,
    report: Option<PathBuf>,
) -> anyhow::Result<()> {
    let exe       = std::env::current_exe()?;
    let bin_dir   = exe.parent().unwrap_or_else(|| std::path::Path::new("."));
    let bench_bin = bin_dir.join("benchmarks");

    // Determine which mode to run.
    let (flag, target): (&str, std::path::PathBuf) = if let Some(ref cfg) = repo {
        let p = cfg.canonicalize().context("cannot resolve --repo path")?;
        ("--repo-bench", p)
    } else if let Some(ref repos_dir) = multi {
        let p = repos_dir.canonicalize().context("cannot resolve --multi path")?;
        ("--multi-bench", p)
    } else {
        let p = path.canonicalize().context("cannot resolve project path")?;
        ("--real-bench", p)
    };

    if bench_bin.exists() {
        let mut cmd = std::process::Command::new(&bench_bin);
        cmd.arg(flag).arg(&target);
        if let Some(ref rp) = report {
            cmd.arg("--report").arg(rp);
        }
        let status = cmd.status().context("failed to launch benchmarks binary")?;
        std::process::exit(status.code().unwrap_or(1));
    }

    // Dev mode: show the equivalent cargo command.
    println!("The benchmarks binary is not installed alongside this binary.");
    println!("Run with:");
    println!();
    let rpt = report.as_ref().map(|p| format!(" --report {}", p.display())).unwrap_or_default();
    println!("  cargo run -p benchmarks --release -- {flag} {}{rpt}", target.display());
    println!();
    if repo.is_none() && multi.is_none() {
        println!("Or run the YAML-driven benchmark (search + multi-hop + impact accuracy):");
        println!("  cargo run -p benchmarks --release -- --repo-bench benchmarks/repos/express.yaml");
        println!();
        println!("Or the synthetic oracle comparison (no project needed):");
        println!("  cargo run -p benchmarks --release -- --bench-tokens --report evidence.html");
        println!();
    }
    Ok(())
}

// ── PID / process helpers ─────────────────────────────────────────────────────

fn read_pid() -> anyhow::Result<Option<u32>> {
    let path = Registry::pid_path();
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    Ok(text.trim().parse::<u32>().ok())
}

fn write_pid(pid: u32) -> anyhow::Result<()> {
    std::fs::create_dir_all(Registry::global_dir())?;
    std::fs::write(Registry::pid_path(), pid.to_string()).context("writing pid file")
}

fn read_port() -> anyhow::Result<u16> {
    let text = std::fs::read_to_string(Registry::port_path())?;
    text.trim().parse::<u16>().context("parsing port file")
}

fn is_process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn send_sigterm(pid: u32) -> anyhow::Result<()> {
    let status = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("failed to send SIGTERM to PID {pid}")
    }
}

// ── compare ───────────────────────────────────────────────────────────────────

fn cmd_compare(
    project_a: String,
    project_b: String,
    max_queries: usize,
    source_dir: Option<PathBuf>,
    report: Option<PathBuf>,
) -> anyhow::Result<()> {
    use graphrelay_parser::{Extractor, LanguageRegistry};

    let registry = Registry::load()?;
    let entry_a = registry.projects.iter()
        .find(|p| p.name == project_a)
        .ok_or_else(|| anyhow::anyhow!(
            "project '{}' not found — run `gr init` from its directory first", project_a
        ))?;
    // Existence check only — unlike `entry_a`, nothing else about project_b's
    // registry entry is needed now that comparisons no longer read a
    // per-project embedding model.
    registry.projects.iter()
        .find(|p| p.name == project_b)
        .ok_or_else(|| anyhow::anyhow!(
            "project '{}' not found — run `gr init` from its directory first", project_b
        ))?;

    let port = read_port().context(
        "daemon is not running — start it first with `gr start`"
    )?;

    let source = source_dir.unwrap_or_else(|| entry_a.root.clone());

    println!("Comparing:");
    println!("  A: {}", project_a);
    println!("  B: {}", project_b);
    println!();
    println!("Extracting docstring pairs from {} …", source.display());

    // Extract docstring (query, fn_name) pairs from the source tree
    const SKIP_DIRS: &[&str] = &[
        "target", ".git", "node_modules", ".graphrelay", "dist", "build",
        "deps", "test", "tests", "benchmarks", "vendor", "__tests__",
        "fixtures", "example", "examples", "docs", "doc",
    ];
    const CODE_EXTS: &[&str] = &["js", "ts", "tsx", "jsx", "rs", "py", "go", "java"];

    let extractor = Extractor::new(LanguageRegistry::default());
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut seen_fns: std::collections::HashSet<String> = Default::default();

    let walker = walkdir::WalkDir::new(&source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or("");
            !SKIP_DIRS.contains(&name)
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() { continue; }
        let ext = entry.path().extension().and_then(|e| e.to_str()).unwrap_or("");
        if !CODE_EXTS.contains(&ext) { continue; }
        let src = match std::fs::read(entry.path()) { Ok(s) => s, Err(_) => continue };
        let result = match extractor.extract(entry.path(), &src) { Ok(r) => r, Err(_) => continue };
        for sym in result.symbols {
            if let Some(ref doc) = sym.docstring {
                if let Some(query) = compare_docstring_to_query(doc) {
                    if seen_fns.insert(sym.name.clone()) {
                        pairs.push((query, sym.name));
                    }
                }
            }
        }
    }

    if pairs.is_empty() {
        println!("  No docstring pairs found.");
        println!("  Try a project with JSDoc, rustdoc, or Python docstrings.");
        return Ok(());
    }

    // Deterministic shuffle, cap at max_queries
    compare_lcg_shuffle(&mut pairs, 42);
    pairs.truncate(max_queries);
    let n = pairs.len();
    println!("  {} pairs (capped at {})", n, max_queries);
    println!();
    println!("Running {} queries against both endpoints …", n);
    println!();

    struct Row {
        query:   String,
        target:  String,
        rank_a:  Option<usize>,
        rank_b:  Option<usize>,
        lat_a:   f64,
        lat_b:   f64,
    }

    let mut rows: Vec<Row> = Vec::with_capacity(n);
    for (i, (query, target)) in pairs.iter().enumerate() {
        eprint!("\r  {}/{} …", i + 1, n);
        let (rank_a, lat_a) = compare_search(port, &project_a, query, target, 10);
        let (rank_b, lat_b) = compare_search(port, &project_b, query, target, 10);
        rows.push(Row { query: query.clone(), target: target.clone(), rank_a, rank_b, lat_a, lat_b });
    }
    eprintln!("\r  done{}", " ".repeat(20));

    // Compute aggregates
    let mrr_a     = compare_mrr(&rows.iter().map(|r| r.rank_a).collect::<Vec<_>>());
    let mrr_b     = compare_mrr(&rows.iter().map(|r| r.rank_b).collect::<Vec<_>>());
    let rec5_a    = compare_recall_at(&rows.iter().map(|r| r.rank_a).collect::<Vec<_>>(), 5);
    let rec5_b    = compare_recall_at(&rows.iter().map(|r| r.rank_b).collect::<Vec<_>>(), 5);
    let rec10_a   = compare_recall_at(&rows.iter().map(|r| r.rank_a).collect::<Vec<_>>(), 10);
    let rec10_b   = compare_recall_at(&rows.iter().map(|r| r.rank_b).collect::<Vec<_>>(), 10);
    let mut lats_a: Vec<f64> = rows.iter().map(|r| r.lat_a).collect();
    let mut lats_b: Vec<f64> = rows.iter().map(|r| r.lat_b).collect();
    lats_a.sort_by(|a, b| a.partial_cmp(b).unwrap());
    lats_b.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50_a = compare_percentile(&lats_a, 0.50);
    let p95_a = compare_percentile(&lats_a, 0.95);
    let p50_b = compare_percentile(&lats_b, 0.50);
    let p95_b = compare_percentile(&lats_b, 0.95);

    // Count per-query winners
    let wins_a = rows.iter().filter(|r| {
        match (r.rank_a, r.rank_b) {
            (Some(a), Some(b)) => a < b,
            (Some(_), None)    => true,
            _                  => false,
        }
    }).count();
    let wins_b = rows.iter().filter(|r| {
        match (r.rank_a, r.rank_b) {
            (Some(a), Some(b)) => b < a,
            (None, Some(_))    => true,
            _                  => false,
        }
    }).count();
    let ties = n - wins_a - wins_b;

    // ── Print per-query table ─────────────────────────────────────────────────
    let q_w = 42usize;
    println!("{:<q_w$}  {:>8}  {:>10}  {:>8}  {:>10}  winner", "Query (→ target)", "A rank", "A lat(ms)", "B rank", "B lat(ms)");
    println!("{}", "-".repeat(q_w + 52));
    for r in &rows {
        let q_display = {
            let short = format!("\"{}\" → {}", &r.query, r.target);
            if short.chars().count() > q_w { short.chars().take(q_w - 1).collect::<String>() + "…" } else { short }
        };
        let rank_a = r.rank_a.map(|x| format!("#{x}")).unwrap_or_else(|| "–".to_string());
        let rank_b = r.rank_b.map(|x| format!("#{x}")).unwrap_or_else(|| "–".to_string());
        let winner = match (r.rank_a, r.rank_b) {
            (Some(a), Some(b)) if a < b => format!("A  ({})", project_a),
            (Some(a), Some(b)) if b < a => format!("B  ({})", project_b),
            (Some(_), None)             => format!("A  ({})", project_a),
            (None, Some(_))             => format!("B  ({})", project_b),
            _                           => "tie".to_string(),
        };
        println!("{:<q_w$}  {:>8}  {:>10.1}  {:>8}  {:>10.1}  {}", q_display, rank_a, r.lat_a, rank_b, r.lat_b, winner);
    }

    println!();
    println!("{:<20}  {:>12}  {:>12}  {:>10}", "Metric", &project_a, &project_b, "Δ (B−A)");
    println!("{}", "-".repeat(58));
    println!("{:<20}  {:>12.4}  {:>12.4}  {:>+10.4}", "MRR", mrr_a, mrr_b, mrr_b - mrr_a);
    println!("{:<20}  {:>12.4}  {:>12.4}  {:>+10.4}", "Recall@5", rec5_a, rec5_b, rec5_b - rec5_a);
    println!("{:<20}  {:>12.4}  {:>12.4}  {:>+10.4}", "Recall@10", rec10_a, rec10_b, rec10_b - rec10_a);
    println!("{:<20}  {:>11.1}ms  {:>11.1}ms", "Latency p50", p50_a, p50_b);
    println!("{:<20}  {:>11.1}ms  {:>11.1}ms", "Latency p95", p95_a, p95_b);
    println!();
    println!("Per-query wins:  A={wins_a}  B={wins_b}  tie={ties}  (n={n})");

    let winner_label = if (mrr_b - mrr_a).abs() < 0.01 {
        "  → tie (MRR delta < 0.01)".to_string()
    } else if mrr_b > mrr_a {
        format!("  → {} wins on MRR (+{:.3})", project_b, mrr_b - mrr_a)
    } else {
        format!("  → {} wins on MRR (+{:.3})", project_a, mrr_a - mrr_b)
    };
    println!("{winner_label}");

    // ── Optional Markdown report ─────────────────────────────────────────────
    if let Some(ref path) = report {
        let mut md = String::new();
        md.push_str(&format!("# Retrieval Comparison: {} vs {}\n\n", project_a, project_b));
        md.push_str(&format!("| | {} | {} | Δ (B−A) |\n", project_a, project_b));
        md.push_str("|---|---|---|---|\n");
        md.push_str(&format!("| MRR | {:.4} | {:.4} | {:+.4} |\n", mrr_a, mrr_b, mrr_b - mrr_a));
        md.push_str(&format!("| Recall@5 | {:.4} | {:.4} | {:+.4} |\n", rec5_a, rec5_b, rec5_b - rec5_a));
        md.push_str(&format!("| Recall@10 | {:.4} | {:.4} | {:+.4} |\n", rec10_a, rec10_b, rec10_b - rec10_a));
        md.push_str(&format!("| Latency p50 | {:.1}ms | {:.1}ms | |\n", p50_a, p50_b));
        md.push_str(&format!("| Latency p95 | {:.1}ms | {:.1}ms | |\n", p95_a, p95_b));
        md.push_str(&format!("| Per-query wins | {wins_a} | {wins_b} | {ties} ties |\n\n"));
        md.push_str("## Per-query breakdown\n\n");
        md.push_str(&format!("| Query | Target | {} rank | {} rank | Winner |\n", project_a, project_b));
        md.push_str("|---|---|---|---|---|\n");
        for r in &rows {
            let rank_a = r.rank_a.map(|x| format!("#{x}")).unwrap_or_else(|| "–".to_string());
            let rank_b = r.rank_b.map(|x| format!("#{x}")).unwrap_or_else(|| "–".to_string());
            let winner = match (r.rank_a, r.rank_b) {
                (Some(a), Some(b)) if a < b => project_a.as_str(),
                (Some(a), Some(b)) if b < a => project_b.as_str(),
                (Some(_), None)             => project_a.as_str(),
                (None, Some(_))             => project_b.as_str(),
                _                           => "tie",
            };
            md.push_str(&format!("| {} | `{}` | {} | {} | {} |\n",
                r.query.replace('|', "\\|"), r.target, rank_a, rank_b, winner));
        }
        std::fs::write(path, &md)
            .with_context(|| format!("writing report to {:?}", path))?;
        println!();
        println!("Report written to {:?}", path);
    }

    Ok(())
}

// ── compare helpers ───────────────────────────────────────────────────────────

fn compare_search(port: u16, project: &str, query: &str, target: &str, top_k: usize)
    -> (Option<usize>, f64)
{
    let start = std::time::Instant::now();
    let body = match ureq_get_search(port, project, query, top_k) {
        Ok(b) => b,
        Err(_) => return (None, start.elapsed().as_secs_f64() * 1000.0),
    };
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let results = v["data"]["results"].as_array();
    let rank = results.and_then(|arr| {
        arr.iter().position(|r| {
            let sym = r["symbol"].as_str().unwrap_or("");
            // Match by short name or qualified name (e.g. "foo" or "Module::foo" or "Class.foo")
            sym == target
                || sym.ends_with(&format!("::{target}"))
                || sym.ends_with(&format!(".{target}"))
                || sym.ends_with(&format!("/{target}"))
        }).map(|i| i + 1)
    });
    (rank, elapsed_ms)
}

fn ureq_get_search(port: u16, project: &str, query: &str, top_k: usize) -> anyhow::Result<String> {
    let base = format!("http://localhost:{}/{}/api/search", port, project);
    let out = std::process::Command::new("curl")
        .args([
            "-sG",
            "--max-time", "120",
            &base,
            "--data-urlencode", &format!("q={query}"),
            "--data-urlencode", &format!("top_k={top_k}"),
        ])
        .output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        anyhow::bail!("curl search failed for project '{}'", project)
    }
}

fn compare_mrr(ranks: &[Option<usize>]) -> f64 {
    if ranks.is_empty() { return 0.0; }
    let sum: f64 = ranks.iter().map(|r| match r {
        Some(k) => 1.0 / *k as f64,
        None    => 0.0,
    }).sum();
    sum / ranks.len() as f64
}

fn compare_recall_at(ranks: &[Option<usize>], k: usize) -> f64 {
    if ranks.is_empty() { return 0.0; }
    let hits = ranks.iter().filter(|r| matches!(r, Some(pos) if *pos <= k)).count();
    hits as f64 / ranks.len() as f64
}

fn compare_percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() { return 0.0; }
    let idx = ((sorted.len() as f64 * p) as usize).min(sorted.len() - 1);
    sorted[idx]
}

fn compare_docstring_to_query(doc: &str) -> Option<String> {
    let bad_line = |l: &str| -> bool {
        l.starts_with('@')
            || l.starts_with('*')
            || l.starts_with("http://")
            || l.starts_with("https://")
            || l.starts_with('<')
            || l.starts_with('{')
            || l.starts_with('[')
            || l.starts_with('|')
            || l.contains("}}")
    };
    let lines: Vec<&str> = doc.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !bad_line(l))
        .collect();
    if lines.is_empty() { return None; }
    let q = lines.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
    let q = q.trim().to_string();
    let word_count = q.split_whitespace()
        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 3)
        .count();
    if q.len() < 20 || word_count < 4 { return None; }
    Some(q.chars().take(120).collect())
}

fn compare_lcg_shuffle<T>(v: &mut [T], seed: u64) {
    let mut s = seed;
    for i in (1..v.len()).rev() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = (s >> 33) as usize % (i + 1);
        v.swap(i, j);
    }
}

// ── minimal HTTP GET for `status` (no extra deps) ────────────────────────────

fn ureq_get(url: &str) -> anyhow::Result<String> {
    // Use std::process + curl so we don't pull in an HTTP client dep
    let out = std::process::Command::new("curl")
        .args(["-s", "--max-time", "2", url])
        .output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        anyhow::bail!("curl failed")
    }
}

// ── shutdown signal ───────────────────────────────────────────────────────────

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let sigterm = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c  => {}
        _ = sigterm => {}
    }
}

// ── Plan 12 D1+D2: SCIP indexer detection and runner ─────────────────────────

/// Check if a binary exists in the extended PATH (including local SCIP install dirs).
/// Extends PATH with local npm-packages and ~/.local/bin for SCIP indexers.
fn find_in_extended_path(binary: &str) -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    let dirs = [
        format!("{home}/.local/npm-packages/node_modules/.bin"),
        format!("{home}/.local/bin"),
        format!("{home}/.cargo/bin"),
    ];
    // Check local dirs first
    for dir in &dirs {
        if std::path::Path::new(dir).join(binary).exists() {
            return true;
        }
    }
    // Fall back to system PATH
    which::which(binary).is_ok()
}

fn extended_scip_path() -> std::ffi::OsString {
    let home = std::env::var("HOME").unwrap_or_default();
    let extra = [
        format!("{home}/.local/npm-packages/node_modules/.bin"),  // scip-python, scip-typescript
        format!("{home}/.local/bin"),                              // scip-java
        format!("{home}/.cargo/bin"),                              // rust-analyzer
    ];
    // `join_paths` treats each element as a single directory component and
    // rejects any element containing the platform path-list separator
    // (`:` on Unix) as ambiguous. `$PATH` itself is already a
    // separator-joined list, not a single directory, so it must be split
    // into its individual entries with `split_paths` before being merged
    // back in — pushing the raw `$PATH` string as one element here used to
    // make `join_paths` fail on every real system (any `$PATH` with more
    // than one entry), and `.unwrap_or_default()` silently swallowed that
    // into an empty PATH, so no extended-path SCIP indexer (or anything
    // else using this path) could ever actually be found by `which_in`.
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<std::ffi::OsString> = extra.iter()
        .map(std::ffi::OsString::from)
        .collect();
    parts.extend(std::env::split_paths(&existing).map(std::ffi::OsString::from));
    std::env::join_paths(parts).unwrap_or_else(|e| {
        tracing::warn!("failed to build extended SCIP PATH: {e}");
        existing
    })
}

const SCIP_INDEXERS: &[(&str, &str, &str)] = &[
    ("python",     "scip-python",            "pip install scip-python"),
    ("typescript", "scip-typescript",        "npm i -g @sourcegraph/scip-typescript"),
    ("java",       "scip-java",              "download from github.com/sourcegraph/scip-java"),
    ("go",         "scip-go",                "go install github.com/sourcegraph/scip-go@latest"),
    ("rust",       "rust-analyzer",          "rustup component add rust-analyzer"),
    ("kotlin",     "scip-kotlin",            "JetBrains Kotlin plugin"),
    ("ruby",       "scip-ruby",              "gem install scip-ruby"),
];

// ── setup ─────────────────────────────────────────────────────────────────────

enum McpTarget {
    /// Claude Code: `claude mcp add` when the CLI exists, else a project `.mcp.json`.
    ClaudeCode,
    /// A JSON file with a top-level `mcpServers` map (Cursor).
    McpServersFile(PathBuf),
    /// A project `opencode.json` with a top-level `mcp` map.
    OpenCodeProject(PathBuf),
    Manual,
}

struct ToolDef {
    id:          &'static str,
    label:       &'static str,
    detected:    bool,
    mcp:         McpTarget,
    instr_files: Vec<PathBuf>,
}

const CLAUDE_SERVER_NAME: &str = "graphrelay";

fn cmd_setup(global: bool, dry_run: bool, tool_filter: Option<String>, all: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let home_str = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let home = std::path::Path::new(&home_str);

    let registry = Registry::load()?;
    let entry = registry.find_by_root(&cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "Current directory is not a registered project.\n\
             Run `gr init` first."
        )
    })?;
    let slug = &entry.name;
    let port = read_port().unwrap_or(3030);
    let mcp_url = format!("http://localhost:{port}/{slug}/mcp");

    // ── Tool catalogue ────────────────────────────────────────────────────────
    let catalogue: Vec<ToolDef> = vec![
        ToolDef {
            id:       "claude",
            label:    "Claude Code",
            detected: which::which("claude").is_ok() || home.join(".claude").exists(),
            mcp:      McpTarget::ClaudeCode,
            instr_files: vec![cwd.join("CLAUDE.md")],
        },
        ToolDef {
            id:       "cursor",
            label:    "Cursor",
            detected: home.join(".cursor").exists(),
            mcp:      McpTarget::McpServersFile(home.join(".cursor").join("mcp.json")),
            instr_files: vec![cwd.join(".cursorrules")],
        },
        ToolDef {
            id:       "opencode",
            label:    "OpenCode",
            detected: which::which("opencode").is_ok(),
            mcp:      McpTarget::OpenCodeProject(cwd.join("opencode.json")),
            instr_files: vec![cwd.join("AGENTS.md")],
        },
        ToolDef {
            id:       "amp",
            label:    "Amp",
            detected: which::which("amp").is_ok(),
            mcp:      McpTarget::Manual,
            instr_files: vec![cwd.join("AGENTS.md")],
        },
        ToolDef {
            id:       "codex",
            label:    "Codex",
            detected: which::which("codex").is_ok(),
            mcp:      McpTarget::Manual,
            instr_files: vec![cwd.join("AGENTS.md")],
        },
    ];

    // ── Resolve which tools to configure ─────────────────────────────────────

    // 1. `--tool <id>` — skip the picker entirely.
    if let Some(ref f) = tool_filter {
        let idx = catalogue.iter().position(|t| t.id == f.as_str())
            .ok_or_else(|| anyhow::anyhow!(
                "Unknown tool '{f}'. Supported: claude, cursor, opencode, amp, codex"
            ))?;
        let tool_ref: Vec<&ToolDef> = vec![&catalogue[idx]];
        return run_setup_for(&tool_ref, slug, &mcp_url, home, &cwd, global, dry_run);
    }

    // 2. `--all`, or no TTY (CI / pipes) — auto-select detected tools.
    if all || !std::io::stdin().is_terminal() {
        let detected: Vec<&ToolDef> = catalogue.iter().filter(|t| t.detected).collect();
        if detected.is_empty() {
            println!("No supported coding tools detected.");
            print_manual_hint(&mcp_url);
            return Ok(());
        }
        return run_setup_for(&detected, slug, &mcp_url, home, &cwd, global, dry_run);
    }

    // 3. Interactive multi-select picker.
    println!();
    println!("  GraphRelay Setup — MCP server + instruction hook");
    println!("  Project: {slug}  ·  MCP: {mcp_url}");
    println!();

    let labels: Vec<String> = catalogue.iter().map(|t| {
        if t.detected {
            format!("{:<16}  (detected)", t.label)
        } else {
            t.label.to_string()
        }
    }).collect();
    let defaults: Vec<bool> = catalogue.iter().map(|t| t.detected).collect();

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select tools to configure  [space = toggle, enter = confirm]")
        .items(&labels)
        .defaults(&defaults)
        .interact();

    let indices = match selections {
        Ok(v) => v,
        Err(dialoguer::Error::IO(e)) if e.kind() == std::io::ErrorKind::Interrupted => {
            println!("Cancelled.");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    if indices.is_empty() {
        println!("Nothing selected — exiting without changes.");
        return Ok(());
    }

    let selected: Vec<&ToolDef> = indices.iter().map(|&i| &catalogue[i]).collect();
    println!();
    run_setup_for(&selected, slug, &mcp_url, home, &cwd, global, dry_run)
}

fn run_setup_for(
    tools:   &[&ToolDef],
    slug:    &str,
    mcp_url: &str,
    home:    &std::path::Path,
    cwd:     &std::path::Path,
    global:  bool,
    dry_run: bool,
) -> anyhow::Result<()> {
    if dry_run {
        println!("(dry-run — nothing will be written)");
        println!();
    }

    let mut seen_mcp:   std::collections::HashSet<PathBuf> = Default::default();
    let mut seen_instr: std::collections::HashSet<PathBuf> = Default::default();

    // ── MCP config ────────────────────────────────────────────────────────────
    println!("MCP server  ({slug} → {mcp_url}):");
    for t in tools {
        match &t.mcp {
            McpTarget::Manual => {
                println!("  {} (skip): MCP injection not yet supported for this tool", t.label);
                println!("    Add manually: {mcp_url}");
            }
            McpTarget::ClaudeCode => match setup_claude_mcp(cwd, mcp_url, dry_run) {
                Ok((changed, how)) => println!(
                    "  ✓ {}  {}  ({how})",
                    t.label,
                    if changed { "registered" } else { "already configured" }
                ),
                Err(e) => {
                    println!("  ✗ {}  error: {e}", t.label);
                    println!("    Run manually: claude mcp add --transport http {CLAUDE_SERVER_NAME} {mcp_url}");
                }
            },
            McpTarget::McpServersFile(path) => {
                if !seen_mcp.insert(path.clone()) { continue; }
                match setup_inject_mcp(path, slug, mcp_url, dry_run) {
                    Ok(true)  => println!("  ✓ {}  →  {}", t.label, path.display()),
                    Ok(false) => println!("  ✓ {}  (already configured)", t.label),
                    Err(e)    => println!("  ✗ {}  error: {e}", t.label),
                }
            }
            McpTarget::OpenCodeProject(path) => {
                if !seen_mcp.insert(path.clone()) { continue; }
                match setup_inject_opencode_mcp(path, CLAUDE_SERVER_NAME, mcp_url, dry_run) {
                    Ok(true)  => println!("  ✓ {}  →  {}", t.label, path.display()),
                    Ok(false) => println!("  ✓ {}  (already configured)", t.label),
                    Err(e)    => println!("  ✗ {}  error: {e}", t.label),
                }
            }
        }
    }
    println!();

    // ── Instruction hook ──────────────────────────────────────────────────────
    println!("Instruction hook  (GraphRelay-first behavior):");
    for t in tools {
        for instr_path in &t.instr_files {
            let dest = if global {
                match t.id {
                    "claude" => home.join(".claude").join("CLAUDE.md"),
                    _        => home.join(".config").join("graphrelay").join("AGENTS.md"),
                }
            } else {
                instr_path.clone()
            };

            if !seen_instr.insert(dest.clone()) { continue; }

            match setup_inject_hook(&dest, dry_run) {
                Ok(true)  => println!("  ✓ {}  →  {}", t.label, dest.display()),
                Ok(false) => println!("  ✓ {}  (already configured)", t.label),
                Err(e)    => println!("  ✗ {}  error: {e}", t.label),
            }
        }
    }

    println!();
    if dry_run {
        println!("dry-run complete — no files were written.");
    } else {
        println!("Setup complete.");
        let port = mcp_url.rsplit_once("://").and_then(|(_, r)| r.split('/').next()).and_then(|h| h.rsplit_once(':')).and_then(|(_, p)| p.parse::<u16>().ok());
        let reachable = port.is_some_and(|p| {
            std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], p)), std::time::Duration::from_millis(500)).is_ok()
        });
        if reachable {
            println!("GraphRelay server is running. Restart your coding tool to pick up the MCP server.");
        } else {
            println!("⚠ The GraphRelay server is not running on {mcp_url}.");
            println!("  Start it with `gr start`; until then the tools will not connect and agents fall back to Grep/Read.");
        }
        if tools.iter().any(|t| matches!(t.mcp, McpTarget::ClaudeCode)) {
            println!("Check the connection with `claude mcp list` (graphrelay should show Connected),");
            println!("or `/mcp` inside Claude Code. If it shows failed, the server is not running.");
        }
        println!("Then try: \"How does authentication work in this project?\"");
        println!("You should see search_code called before any file reads.");
    }
    Ok(())
}

fn print_manual_hint(mcp_url: &str) {
    println!();
    println!("  Supported: Claude Code, Cursor, OpenCode, Amp, Codex");
    println!("  After installing one, re-run `gr setup`.");
    println!();
    println!("  To configure manually, add this MCP URL to your tool's config:");
    println!("    {mcp_url}");
}

fn setup_inject_mcp(
    config_path: &std::path::Path,
    slug:        &str,
    mcp_url:     &str,
    dry_run:     bool,
) -> anyhow::Result<bool> {
    let mut json: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("reading {}", config_path.display()))?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Idempotency check
    let existing_url = json.get("mcpServers").and_then(|s| s.get(slug)).and_then(|e| e.get("url")).and_then(|u| u.as_str());
    if existing_url == Some(mcp_url) {
        return Ok(false);
    }

    let obj = json
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("MCP config root is not a JSON object"))?;

    let servers = obj
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}));

    if let Some(srv) = servers.as_object_mut() {
        srv.insert(slug.to_string(), serde_json::json!({
            "type": "http",
            "url":  mcp_url,
        }));
    }
    write_json_config(config_path, &json, dry_run)
}

fn write_json_config(config_path: &std::path::Path, json: &serde_json::Value, dry_run: bool) -> anyhow::Result<bool> {

    if dry_run {
        return Ok(true);
    }

    if config_path.exists() {
        let bak = config_path.with_extension("json.bak");
        std::fs::copy(config_path, &bak)
            .with_context(|| format!("backup to {}", bak.display()))?;
    }
    if let Some(dir) = config_path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(config_path, serde_json::to_string_pretty(json)?)
        .with_context(|| format!("writing {}", config_path.display()))?;

    Ok(true)
}

fn setup_inject_opencode_mcp(
    config_path: &std::path::Path,
    name:        &str,
    mcp_url:     &str,
    dry_run:     bool,
) -> anyhow::Result<bool> {
    let mut json: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("reading {}", config_path.display()))?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("{} is not plain JSON ({e}); add the server by hand", config_path.display()))?
    } else {
        serde_json::json!({ "$schema": "https://opencode.ai/config.json" })
    };

    let existing_url = json.get("mcp").and_then(|m| m.get(name)).and_then(|e| e.get("url")).and_then(|u| u.as_str());
    if existing_url == Some(mcp_url) {
        return Ok(false);
    }

    let obj = json
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("OpenCode config root is not a JSON object"))?;
    let servers = obj
        .entry("mcp".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if let Some(srv) = servers.as_object_mut() {
        srv.insert(name.to_string(), serde_json::json!({
            "type":    "remote",
            "url":     mcp_url,
            "enabled": true,
        }));
    }
    write_json_config(config_path, &json, dry_run)
}

/// Registers the MCP server with Claude Code. Claude Code does not read
/// `~/.claude/claude_desktop_config.json` (that is Claude Desktop's file); it keeps
/// servers in `~/.claude.json` / `.mcp.json`, managed through `claude mcp`.
fn setup_claude_mcp(cwd: &std::path::Path, mcp_url: &str, dry_run: bool) -> anyhow::Result<(bool, &'static str)> {
    if which::which("claude").is_err() {
        let path = cwd.join(".mcp.json");
        let changed = setup_inject_mcp(&path, CLAUDE_SERVER_NAME, mcp_url, dry_run)?;
        return Ok((changed, "claude CLI not found; wrote .mcp.json, approve it on first use"));
    }

    let run = |args: &[&str]| std::process::Command::new("claude").args(args).current_dir(cwd).output();

    if let Ok(out) = run(&["mcp", "get", CLAUDE_SERVER_NAME]) {
        if out.status.success() {
            if String::from_utf8_lossy(&out.stdout).contains(mcp_url) {
                return Ok((false, "claude mcp"));
            }
            if !dry_run {
                run(&["mcp", "remove", CLAUDE_SERVER_NAME, "--scope", "local"])?;
            }
        }
    }
    if dry_run {
        return Ok((true, "claude mcp add"));
    }
    let out = run(&["mcp", "add", "--transport", "http", "--scope", "local", CLAUDE_SERVER_NAME, mcp_url])?;
    if !out.status.success() {
        anyhow::bail!("`claude mcp add` failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok((true, "claude mcp add, local scope"))
}

// Plan 23: deliberately NOT one of the tool names below — a marker tied to a
// specific tool name silently breaks `gr init`'s "already injected"
// detection the next time a tool gets renamed (as happened here: the old
// marker, "find_relevant_code", no longer appears in a freshly-injected
// block at all). The block's own stable heading text doesn't have that
// failure mode.
const SETUP_HOOK_MARKER: &str = "## Code search — use GraphRelay first";

const SETUP_HOOK_BLOCK: &str = "\
\n\
## Code search — use GraphRelay first\n\
\n\
Before reading files, grepping, or exploring the codebase, always query GraphRelay MCP\n\
tools. GraphRelay returns ranked symbols with call graphs in one tool call instead of\n\
many file reads — this reduces token usage and improves accuracy.\n\
\n\
- `search_code`  — intent-based search (ranked symbols + snippets + call graphs) or\n\
  named-type lookup (find every implementation of an interface/trait/base class)\n\
- `inspect`      — a known symbol's definition + callers/callees, or a file's/module's\n\
  contents (pass `include_source: true` for a file's full text, in place of Read)\n\
- `trace_impact` — blast-radius before changing anything\n\
- `watch_status` — check index freshness\n\
\n\
In Claude Code these appear as `mcp__graphrelay__search_code`, `mcp__graphrelay__inspect`, etc.\n\
If they are not available, say so and ask the user to run `gr start` and `gr setup`\n\
(check with `claude mcp list`) instead of silently falling back to Grep/Read.\n\
\n\
Only fall back to direct file reads when GraphRelay cannot answer (config files,\n\
non-indexed content, binary files).\n";

fn setup_inject_hook(dest: &std::path::Path, dry_run: bool) -> anyhow::Result<bool> {
    if dest.exists() {
        let content = std::fs::read_to_string(dest)
            .with_context(|| format!("reading {}", dest.display()))?;
        if content.contains(SETUP_HOOK_MARKER) {
            return Ok(false);
        }
    }

    if dry_run {
        return Ok(true);
    }

    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }

    let mut content = if dest.exists() {
        std::fs::read_to_string(dest)?
    } else {
        String::new()
    };
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(SETUP_HOOK_BLOCK);

    std::fs::write(dest, &content)
        .with_context(|| format!("writing {}", dest.display()))?;

    Ok(true)
}

fn cmd_scip(scip_cmd: ScipCommand) -> anyhow::Result<()> {
    match scip_cmd {
        ScipCommand::Detect => cmd_scip_detect(),
        ScipCommand::Index { lang, output } => cmd_scip_index(&lang, &output),
    }
}

fn cmd_scip_detect() -> anyhow::Result<()> {
    println!("SCIP indexer availability:");
    for (lang, binary, hint) in SCIP_INDEXERS {
        let found = find_in_extended_path(binary);
        let status = if found { "✓" } else { "✗" };
        if found {
            println!("  {status} {lang:<12} {binary}");
        } else {
            println!("  {status} {lang:<12} {binary}  (install: {hint})");
        }
    }
    Ok(())
}

fn cmd_scip_index(lang: &str, output: &std::path::Path) -> anyhow::Result<()> {
    // Ensure output directory exists
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating output directory {}", parent.display()))?;
    }

    let (binary, args) = match lang {
        "python" | "py" => (
            "scip-python",
            vec!["index".to_string(), "--output".to_string(), output.to_string_lossy().to_string()],
        ),
        "typescript" | "ts" | "javascript" | "js" => (
            "scip-typescript",
            vec!["index".to_string(), "--output".to_string(), output.to_string_lossy().to_string()],
        ),
        "java" => (
            "scip-java",
            vec!["index".to_string(), "--output".to_string(), output.to_string_lossy().to_string()],
        ),
        "go" => (
            "scip-go",
            vec!["--output".to_string(), output.to_string_lossy().to_string()],
        ),
        "rust" => (
            "rust-analyzer",
            vec!["scip".to_string(), ".".to_string()],
        ),
        _ => anyhow::bail!(
            "Unknown language '{}'. Supported: python, typescript, javascript, java, go, rust",
            lang
        ),
    };

    // Check binary exists in extended PATH (includes ~/.local/npm-packages and ~/.local/bin)
    let scip_path = extended_scip_path();
    let found_bin = which::which_in(binary, Some(&scip_path), std::env::current_dir().unwrap_or_default())
        .with_context(|| format!(
            "{binary} not found. See `gr scip detect` for install instructions."
        ))?;

    println!("Running: {} {}", found_bin.display(), args.join(" "));
    let status = std::process::Command::new(&found_bin)
        .args(&args)
        .env("PATH", &scip_path)
        .status()
        .with_context(|| format!("failed to run {binary}"))?;

    anyhow::ensure!(
        status.success(),
        "{binary} exited with non-zero status: {}. Check that project dependencies are installed.",
        status
    );

    println!("✓ SCIP index written to {}", output.display());
    println!("  Run `gr restart` to trigger automatic ingest.");
    Ok(())
}

#[cfg(test)]
mod setup_tests {
    use super::*;

    #[test]
    fn mcp_servers_file_is_written_then_idempotent_then_updated_on_url_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        assert!(setup_inject_mcp(&path, "proj", "http://localhost:3030/proj/mcp", false).unwrap());
        assert!(!setup_inject_mcp(&path, "proj", "http://localhost:3030/proj/mcp", false).unwrap());
        assert!(setup_inject_mcp(&path, "proj", "http://localhost:4040/proj/mcp", false).unwrap());
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["mcpServers"]["proj"]["url"], "http://localhost:4040/proj/mcp");
        assert_eq!(json["mcpServers"]["proj"]["type"], "http");
    }

    #[test]
    fn mcp_servers_file_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        assert!(setup_inject_mcp(&path, "proj", "http://localhost:3030/proj/mcp", true).unwrap());
        assert!(!path.exists());
    }

    #[test]
    fn opencode_config_uses_the_mcp_remote_shape_and_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.json");
        std::fs::write(&path, r#"{"theme":"dark","mcp":{"other":{"type":"local","command":["x"]}}}"#).unwrap();
        assert!(setup_inject_opencode_mcp(&path, "graphrelay", "http://localhost:3030/p/mcp", false).unwrap());
        assert!(!setup_inject_opencode_mcp(&path, "graphrelay", "http://localhost:3030/p/mcp", false).unwrap());
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["theme"], "dark");
        assert_eq!(json["mcp"]["other"]["type"], "local");
        assert_eq!(json["mcp"]["graphrelay"]["type"], "remote");
        assert_eq!(json["mcp"]["graphrelay"]["url"], "http://localhost:3030/p/mcp");
        assert_eq!(json["mcp"]["graphrelay"]["enabled"], true);
    }

    #[test]
    fn opencode_config_that_is_not_plain_json_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.json");
        std::fs::write(&path, "{ // comment\n}").unwrap();
        assert!(setup_inject_opencode_mcp(&path, "graphrelay", "http://x/mcp", false).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ // comment\n}");
    }

    #[test]
    fn hook_block_is_appended_once_and_mentions_the_prefixed_tool_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        assert!(setup_inject_hook(&path, false).unwrap());
        assert!(!setup_inject_hook(&path, false).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches(SETUP_HOOK_MARKER).count(), 1);
        assert!(text.contains("mcp__graphrelay__search_code"));
    }
}
