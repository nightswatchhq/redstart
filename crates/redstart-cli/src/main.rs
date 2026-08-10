//! The `redstart` command-line toolchain.
//!
//! A single binary, Gleam-style: `new`, `check`, `build`. More subcommands
//! (`dev`, `test`, `fmt`, `lsp`) follow in later stages.

#![forbid(unsafe_code)]

use clap::{Parser, Subcommand};
use redstart_parser::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Redstart: a language for authoring The Graph subgraphs.
#[derive(Parser)]
#[command(name = "redstart", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a new Redstart project.
    New {
        /// The project name (a directory of this name is created).
        name: String,
    },
    /// Parse and validate a project without emitting artifacts.
    Check {
        /// Path to a project directory, `redstart.toml`, or a `.red` file.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit diagnostics as JSON (for editors and agent loops).
        #[arg(long)]
        json: bool,
    },
    /// Build a project: emit schema.graphql, subgraph.yaml, and mappings.ts.
    Build {
        /// Path to a project directory, `redstart.toml`, or a `.red` file.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Run `test` blocks natively against a mock store (no WASM, no Docker).
    Test {
        /// Path to a project directory, `redstart.toml`, or a `.red` file.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Watch `.red` files and re-run check → build → test on every change.
    Dev {
        /// Path to a project directory, `redstart.toml`, or a `.red` file.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Run the pipeline once and exit (handy for CI).
        #[arg(long)]
        once: bool,
    },
    /// Format `.red` files into the canonical layout.
    Fmt {
        /// A `.red` file or a directory to format recursively.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Check formatting without writing; exit non-zero if any file differs.
        #[arg(long)]
        check: bool,
    },
    /// Build, then run `graph codegen` + `graph build` + `graph deploy`.
    ///
    /// Wraps the canonical toolchain end-to-end: Redstart emits the subgraph,
    /// `graph build` compiles it to WASM (the eject path), and `graph deploy`
    /// ships it to Subgraph Studio or a self-hosted graph-node.
    Deploy {
        /// The subgraph name / Studio slug to deploy as.
        subgraph: String,
        /// Path to a project directory, `redstart.toml`, or a `.red` file.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// graph-node admin endpoint (omit for Subgraph Studio).
        #[arg(long)]
        node: Option<String>,
        /// IPFS endpoint (omit for Subgraph Studio's default).
        #[arg(long)]
        ipfs: Option<String>,
        /// Version label for the deployment (e.g. `v0.0.1`).
        #[arg(long)]
        version_label: Option<String>,
        /// Emit the subgraph and run codegen + build, but stop before deploying.
        #[arg(long)]
        dry_run: bool,
    },
    /// Apply an opt-in mechanical rewrite to `.red` source.
    ///
    /// `redstart fix --ids` steers `String`-keyed entities to a `Bytes` id
    /// (`Id<Bytes>` — faster to index, roughly half the disk) by flipping the
    /// schema declaration and dropping the `.toHexString()` at every construction
    /// site. It only converts an entity when *every* one of its id sites is a
    /// single stringified address/bytes value; otherwise the entity is reported
    /// and left untouched. Because a `Bytes` id changes the stored id value
    /// (hex-string → raw bytes) this is a deliberate data change — hence opt-in.
    Fix {
        /// Path to a project directory, `redstart.toml`, or a `.red` file.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Rewrite `String` ids to `Bytes` ids (roadmap §4.3).
        #[arg(long)]
        ids: bool,
        /// Show the changes without writing them.
        #[arg(long)]
        dry_run: bool,
    },
    /// Build, then prove the output compiles: `graph codegen` + `graph build`.
    ///
    /// `redstart check` answers "is this Redstart valid?"; `verify` answers the
    /// question that actually matters before a deploy — "does the generated
    /// subgraph compile to WASM?" Needs Node and npm; the canonical toolchain is
    /// installed into the build directory on first run.
    Verify {
        /// Path to a project directory, `redstart.toml`, or a `.red` file.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Explain a diagnostic code — what it means, the bug it prevents, the fix.
    ///
    /// `redstart explain E062`, or `redstart explain` to list every code.
    Explain {
        /// The diagnostic code, e.g. `E062` (omit to list all codes).
        code: Option<String>,
        /// Emit the explanation as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Start the language server over stdio (for editor integration).
    Lsp,
    /// Start the MCP server over stdio (author-side tools for AI agents).
    ///
    /// Exposes `check`, `explain`, `build`, and `test` as Model Context Protocol
    /// tools, so an agent can drive the toolchain in a write → check → fix loop.
    Mcp,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::New { name } => cmd_new(&name),
        Command::Check { path, json } => cmd_check(&path, json),
        Command::Build { path } => cmd_build(&path),
        Command::Test { path } => cmd_test(&path),
        Command::Dev { path, once } => cmd_dev(&path, once),
        Command::Fmt { path, check } => cmd_fmt(&path, check),
        Command::Deploy {
            subgraph,
            path,
            node,
            ipfs,
            version_label,
            dry_run,
        } => cmd_deploy(
            &path,
            &subgraph,
            node.as_deref(),
            ipfs.as_deref(),
            version_label.as_deref(),
            dry_run,
        ),
        Command::Verify { path } => cmd_verify(&path),
        Command::Fix { path, ids, dry_run } => cmd_fix(&path, ids, dry_run),
        Command::Explain { code, json } => cmd_explain(code.as_deref(), json),
        Command::Lsp => {
            redstart_lsp::run();
            Ok(())
        }
        Command::Mcp => {
            redstart_mcp::run();
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            // An empty message means the command already reported the failure
            // (e.g. `check --json` printed a JSON diagnostics array to stdout).
            if !msg.is_empty() {
                eprintln!("{msg}");
            }
            ExitCode::FAILURE
        }
    }
}

fn cmd_check(path: &Path, json: bool) -> Result<(), String> {
    if json {
        return cmd_check_json(path);
    }
    let tree = load(path)?;
    let diags = redstart_checker::check_diags(&tree);
    let errors = diags.iter().filter(|d| d.is_error()).count();
    let warnings = diags.len() - errors;

    // Print every diagnostic (errors and warnings), errors-first.
    for d in diags.iter().filter(|d| d.is_error()) {
        eprint!("{}", d.render());
    }
    for d in diags.iter().filter(|d| !d.is_error()) {
        eprint!("{}", d.render());
    }

    if errors > 0 {
        // Already printed above; fail without a duplicate message.
        return Err(String::new());
    }

    let modules = tree.modules.len();
    let handlers: usize = tree
        .modules
        .values()
        .map(|m| m.program.handlers.len())
        .sum();
    let warn_note = if warnings > 0 {
        format!(", {warnings} warning(s)")
    } else {
        String::new()
    };
    println!(
        "✓ {} — {modules} module(s), {handlers} handler(s), no errors{warn_note}",
        tree.name
    );
    Ok(())
}

/// `check --json`: emit a machine-readable diagnostics document to stdout, for
/// editors and agent loops. Shape: `{ "ok": bool, "diagnostics": [ {code,
/// severity, message, help, file, line, column, offset, length}, … ] }`.
/// Exits non-zero (with an empty error, already-reported) when not `ok`.
fn cmd_check_json(path: &Path) -> Result<(), String> {
    // Load (lex/parse/resolution) errors first — these block checking.
    let tree = match redstart_loader::load(path) {
        Ok(tree) => tree,
        Err(errors) => {
            let diags: Vec<_> = errors
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "severity": "error",
                        "code": load_error_code(e),
                        "message": strip_ansi(&e.to_string()),
                    })
                })
                .collect();
            print_json_doc(false, &diags);
            return Err(String::new());
        }
    };

    let found = redstart_checker::check_diags(&tree);
    let diags: Vec<_> = found
        .iter()
        .map(|d| {
            serde_json::json!({
                "severity": d.severity_str(),
                "code": d.code_short(),
                "message": d.message,
                "label": d.label_str(),
                "help": d.help_str(),
                "file": d.file,
                "line": d.line,
                "column": d.col,
                "offset": d.offset,
                "length": d.len,
            })
        })
        .collect();
    // `ok` (and the exit code) reflect errors only — warnings don't fail a build.
    let ok = found.iter().all(|d| !d.is_error());
    print_json_doc(ok, &diags);
    if ok {
        Ok(())
    } else {
        Err(String::new())
    }
}

fn print_json_doc(ok: bool, diagnostics: &[serde_json::Value]) {
    let doc = serde_json::json!({ "ok": ok, "diagnostics": diagnostics });
    println!(
        "{}",
        serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
    );
}

/// A short stage code for a loader error, for `--json` output.
fn load_error_code(e: &redstart_loader::LoadError) -> &'static str {
    use redstart_loader::LoadError;
    match e {
        LoadError::Lex { .. } => "lex",
        LoadError::Parse { .. } => "parse",
        LoadError::CircularDependency { .. } => "cycle",
        LoadError::ModuleNotFound { .. } | LoadError::AmbiguousModule { .. } => "module",
        _ => "load",
    }
}

/// Strip ANSI escape sequences so rendered reports are clean inside JSON strings.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip until the terminating letter of the escape (e.g. `m`).
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `redstart explain [CODE]`: explain a diagnostic code, or list all of them.
fn cmd_explain(code: Option<&str>, json: bool) -> Result<(), String> {
    match code {
        None => {
            if json {
                let arr: Vec<_> = redstart_checker::explain::all()
                    .iter()
                    .map(explanation_json)
                    .collect();
                print_pretty(&serde_json::json!({ "codes": arr }));
            } else {
                println!("Redstart diagnostic codes:\n");
                for e in redstart_checker::explain::all() {
                    println!("  \x1b[1m{}\x1b[0m  {}", e.code, e.title);
                }
                println!("\nRun `redstart explain <CODE>` for details on any one.");
            }
            Ok(())
        }
        Some(c) => {
            let e = redstart_checker::explain::explain(c).ok_or_else(|| {
                format!("unknown diagnostic code `{c}` — run `redstart explain` to list all codes")
            })?;
            if json {
                print_pretty(&explanation_json(e));
            } else {
                println!("\x1b[1;31m{}\x1b[0m — {}\n", e.code, e.title);
                println!("{}\n", e.summary);
                if !e.prevents.is_empty() {
                    println!("\x1b[1mPrevents\x1b[0m  {}\n", e.prevents);
                }
                println!("\x1b[1mFix\x1b[0m       {}", e.fix);
            }
            Ok(())
        }
    }
}

fn explanation_json(e: &redstart_checker::Explanation) -> serde_json::Value {
    serde_json::json!({
        "code": e.code,
        "title": e.title,
        "summary": e.summary,
        "prevents": (!e.prevents.is_empty()).then_some(e.prevents),
        "fix": e.fix,
    })
}

fn print_pretty(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
    );
}

fn cmd_build(path: &Path) -> Result<(), String> {
    let tree = load(path)?;
    let mut checked = check(&tree)?;
    let generated = redstart_codegen::generate(&tree, &mut checked);

    generated.write_to(&tree.out_dir).map_err(|e| {
        format!(
            "failed to write build output to {}: {e}",
            tree.out_dir.display()
        )
    })?;

    for warning in &generated.warnings {
        eprintln!("warning: {warning}");
    }
    for note in &generated.notes {
        println!("\x1b[36m⚡ {note}\x1b[0m");
    }

    println!("✓ built {} → {}", tree.name, tree.out_dir.display());
    println!("  • schema.graphql");
    println!("  • subgraph.yaml");
    println!("  • src/mappings.ts");
    println!("  • abis/ ({} file(s))", generated.abi_copies.len());
    // Say plainly which half of the tree is authored and which is output — in a
    // migration, generated mappings are easily mistaken for retained legacy ones.
    println!(
        "  generated output — don't edit or commit it; the .red sources are the truth. \
         `redstart verify` proves it compiles."
    );
    Ok(())
}

/// Run the canonical toolchain over an emitted subgraph: pin `graph-cli` /
/// `graph-ts`, install them if needed, then `graph codegen` + `graph build`.
///
/// This is the eject path, and the only thing that can honestly answer "will
/// this deploy?" — a Redstart build proves the *source* is valid, not that the
/// AssemblyScript it emitted compiles.
fn compile_eject(out: &Path) -> Result<(), String> {
    let pkg = out.join("package.json");
    if !pkg.exists() {
        std::fs::write(
            &pkg,
            "{\n  \"name\": \"redstart-subgraph\",\n  \"private\": true,\n  \"devDependencies\": {\n    \"@graphprotocol/graph-cli\": \"latest\",\n    \"@graphprotocol/graph-ts\": \"latest\"\n  }\n}\n",
        )
        .map_err(|e| format!("failed to write package.json: {e}"))?;
    }

    if !out.join("node_modules").exists() {
        println!("\x1b[1;36m▸\x1b[0m npm install (graph-cli + graph-ts)");
        run_in(out, "npm", &["install", "--no-audit", "--no-fund"])?;
    }

    println!("\x1b[1;36m▸\x1b[0m graph codegen");
    run_in(
        out,
        "npx",
        &["--no-install", "graph", "codegen", "subgraph.yaml"],
    )?;
    println!("\x1b[1;36m▸\x1b[0m graph build");
    run_in(
        out,
        "npx",
        &["--no-install", "graph", "build", "subgraph.yaml"],
    )
}

/// `redstart verify` — build, then prove the generated subgraph compiles.
fn cmd_verify(path: &Path) -> Result<(), String> {
    let tree = load(path)?;
    let mut checked = check(&tree)?;
    let generated = redstart_codegen::generate(&tree, &mut checked);
    generated.write_to(&tree.out_dir).map_err(|e| {
        format!(
            "failed to write build output to {}: {e}",
            tree.out_dir.display()
        )
    })?;
    for warning in &generated.warnings {
        eprintln!("warning: {warning}");
    }
    let out = &tree.out_dir;
    println!("\x1b[1;36m▸\x1b[0m emitted subgraph → {}", out.display());

    compile_eject(out)?;
    println!(
        "\x1b[1;32m✓\x1b[0m verified — `{}` checks, generates, and compiles to WASM",
        tree.name
    );
    Ok(())
}

fn cmd_deploy(
    path: &Path,
    subgraph: &str,
    node: Option<&str>,
    ipfs: Option<&str>,
    version_label: Option<&str>,
    dry_run: bool,
) -> Result<(), String> {
    // 1. Emit the subgraph (same as `redstart build`).
    let tree = load(path)?;
    let mut checked = check(&tree)?;
    let generated = redstart_codegen::generate(&tree, &mut checked);
    generated.write_to(&tree.out_dir).map_err(|e| {
        format!(
            "failed to write build output to {}: {e}",
            tree.out_dir.display()
        )
    })?;
    for warning in &generated.warnings {
        eprintln!("warning: {warning}");
    }
    let out = &tree.out_dir;
    println!("\x1b[1;36m▸\x1b[0m emitted subgraph → {}", out.display());

    // 2-4. Compile the emitted subgraph with the canonical toolchain.
    compile_eject(out)?;

    if dry_run {
        println!("\x1b[1;32m✓\x1b[0m dry run complete — subgraph compiled; skipped deploy");
        return Ok(());
    }

    // 5. graph deploy.
    let mut args: Vec<String> = vec![
        "--no-install".into(),
        "graph".into(),
        "deploy".into(),
        subgraph.into(),
    ];
    if let Some(n) = node {
        args.push("--node".into());
        args.push(n.into());
    }
    if let Some(i) = ipfs {
        args.push("--ipfs".into());
        args.push(i.into());
    }
    if let Some(v) = version_label {
        args.push("--version-label".into());
        args.push(v.into());
    }
    println!("\x1b[1;36m▸\x1b[0m graph deploy {subgraph}");
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_in(out, "npx", &arg_refs)?;

    println!("\x1b[1;32m✓\x1b[0m deployed {subgraph}");
    Ok(())
}

/// Run `program args…` in `dir`, inheriting stdio, mapping a failure to an error.
fn run_in(dir: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    let status = std::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("could not run `{program}` (is it installed and on PATH?): {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed", args.join(" ")))
    }
}

/// Load a project, formatting any errors into a single message.
fn load(path: &Path) -> Result<redstart_loader::ModuleTree, String> {
    redstart_loader::load(path).map_err(|errors| {
        errors
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n\n")
    })
}

/// Run semantic analysis, joining rendered diagnostics into one message.
fn check(tree: &redstart_loader::ModuleTree) -> Result<redstart_checker::Checked, String> {
    redstart_checker::check(tree).map_err(|reports| reports.join("\n"))
}

fn cmd_test(path: &Path) -> Result<(), String> {
    let tree = load(path)?;
    let checked = check(&tree)?;
    let report = redstart_test::run_tests(&tree, &checked);

    if report.results.is_empty() {
        println!("no tests found (add `test \"…\" {{ … }}` blocks)");
        return Ok(());
    }

    for r in &report.results {
        match &r.outcome {
            redstart_test::Outcome::Pass => println!("  \x1b[32m✓\x1b[0m {}", r.name),
            redstart_test::Outcome::Fail { message, location } => {
                let at = location
                    .as_deref()
                    .map_or(String::new(), |l| format!(" ({l})"));
                println!("  \x1b[31m✗\x1b[0m {}{at}\n      {message}", r.name);
            }
        }
    }

    let total = report.results.len();
    let passed = report.passed();
    if report.ok() {
        println!("\n✓ {passed}/{total} passed");
        Ok(())
    } else {
        Err(format!("\n✗ {}/{total} failed", total - passed))
    }
}

fn cmd_dev(path: &Path, once: bool) -> Result<(), String> {
    if once {
        dev_once(path);
        return Ok(());
    }

    let root = if path.is_file() {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        path.to_path_buf()
    };
    println!(
        "redstart dev — watching {} (Ctrl-C to stop)",
        root.display()
    );

    let mut last = String::new();
    let mut n = 0u64;
    loop {
        let fp = fingerprint(&root);
        if fp != last {
            last = fp;
            n += 1;
            println!("\n\x1b[1;36m── rebuild #{n} ──\x1b[0m");
            dev_once(path);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

/// One pass of the dev pipeline: load → check → build → test, printing each step
/// and never aborting the process (so the watch loop keeps going).
fn dev_once(path: &Path) {
    let tree = match load(path) {
        Ok(t) => t,
        Err(e) => return eprintln!("\x1b[31m✗ load\x1b[0m\n{e}"),
    };
    let mut checked = match check(&tree) {
        Ok(c) => c,
        Err(e) => return eprintln!("\x1b[31m✗ check\x1b[0m\n{e}"),
    };
    println!("  \x1b[32m✓\x1b[0m check");

    let generated = redstart_codegen::generate(&tree, &mut checked);
    match generated.write_to(&tree.out_dir) {
        Ok(()) => {
            for w in &generated.warnings {
                eprintln!("    warning: {w}");
            }
            println!("  \x1b[32m✓\x1b[0m build → {}", tree.out_dir.display());
        }
        Err(e) => return eprintln!("  \x1b[31m✗ build\x1b[0m: {e}"),
    }

    let report = redstart_test::run_tests(&tree, &checked);
    if report.results.is_empty() {
        println!("  \x1b[2m·\x1b[0m no tests");
        return;
    }
    for r in &report.results {
        match &r.outcome {
            redstart_test::Outcome::Pass => println!("  \x1b[32m✓\x1b[0m {}", r.name),
            redstart_test::Outcome::Fail { message, location } => {
                let at = location
                    .as_deref()
                    .map_or(String::new(), |l| format!(" ({l})"));
                println!("  \x1b[31m✗\x1b[0m {}{at}\n      {message}", r.name);
            }
        }
    }
    let (passed, total) = (report.passed(), report.results.len());
    let mark = if report.ok() {
        "\x1b[32m✓\x1b[0m"
    } else {
        "\x1b[31m✗\x1b[0m"
    };
    println!("  {mark} {passed}/{total} tests");
}

/// A change-detection fingerprint: sorted `path|mtime` for every `.red` and
/// `.json` (ABI) file under `root`, skipping the build output and hidden dirs.
fn fingerprint(root: &Path) -> String {
    let mut entries = Vec::new();
    collect_fingerprint(root, &mut entries);
    entries.sort();
    entries.join("\n")
}

fn collect_fingerprint(dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "build" || name.starts_with('.') {
                continue;
            }
            collect_fingerprint(&p, out);
        } else if p.extension().is_some_and(|e| e == "red" || e == "json") {
            let stamp = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            out.push(format!("{}|{stamp}", p.display()));
        }
    }
}

fn cmd_fmt(path: &Path, check: bool) -> Result<(), String> {
    let files = collect_red_files(path)?;
    if files.is_empty() {
        return Err(format!("no `.red` files found at {}", path.display()));
    }

    let mut changed = Vec::new();
    for file in &files {
        let src = std::fs::read_to_string(file)
            .map_err(|e| format!("could not read {}: {e}", file.display()))?;
        let formatted = fmt::format(&src);
        if formatted != src {
            changed.push(file.clone());
            if !check {
                std::fs::write(file, &formatted)
                    .map_err(|e| format!("could not write {}: {e}", file.display()))?;
            }
        }
    }

    if check {
        if changed.is_empty() {
            println!("✓ all {} file(s) already formatted", files.len());
            Ok(())
        } else {
            for f in &changed {
                println!("would reformat {}", f.display());
            }
            Err(format!("{} file(s) need formatting", changed.len()))
        }
    } else {
        if changed.is_empty() {
            println!("✓ {} file(s) already formatted", files.len());
        } else {
            for f in &changed {
                println!("formatted {}", f.display());
            }
        }
        Ok(())
    }
}

/// `redstart fix --ids`: rewrite `String`-keyed entities to `Bytes` ids in place.
fn cmd_fix(path: &Path, ids: bool, dry_run: bool) -> Result<(), String> {
    use std::collections::HashMap;

    if !ids {
        return Err(
            "nothing to do — pass a fixer, e.g. `redstart fix --ids` (see `redstart fix --help`)"
                .to_string(),
        );
    }

    let tree = load(path)?;

    // Autofix operates on source that already checks. Warnings (W040 is the whole
    // point here) are fine; hard errors are not — rewriting broken source is asking
    // for trouble.
    let diags = redstart_checker::check_diags(&tree);
    if diags.iter().any(redstart_checker::Diag::is_error) {
        for d in diags.iter().filter(|d| d.is_error()) {
            eprint!("{}", d.render());
        }
        return Err("fix aborted: resolve the errors above first".to_string());
    }

    let plan = redstart_checker::plan_id_rewrites(&tree);
    if plan.rewrites.is_empty() && plan.skipped.is_empty() {
        println!("✓ no `String` ids to rewrite — nothing to do");
        return Ok(());
    }

    // Group every edit by file so overlapping entities in one file splice cleanly.
    let mut by_file: HashMap<String, Vec<(usize, usize, String)>> = HashMap::new();
    for r in &plan.rewrites {
        println!(
            "  ✓ {}  ({} site{}) → Id<Bytes>",
            r.entity,
            r.sites,
            if r.sites == 1 { "" } else { "s" }
        );
        for e in &r.edits {
            by_file.entry(e.file.clone()).or_default().push((
                e.start,
                e.end,
                e.replacement.clone(),
            ));
        }
    }
    for s in &plan.skipped {
        println!(
            "  ⤫ {}  skipped: {} ({}:{})",
            s.entity, s.reason, s.file, s.line
        );
    }

    if plan.rewrites.is_empty() {
        println!("\nnothing to rewrite — every candidate entity was skipped");
        return Ok(());
    }

    if dry_run {
        let files = by_file.len();
        println!(
            "\n{} entit{} across {file_word} — run without --dry-run to apply",
            plan.rewrites.len(),
            if plan.rewrites.len() == 1 { "y" } else { "ies" },
            file_word = if files == 1 {
                "1 file".to_string()
            } else {
                format!("{files} files")
            }
        );
        return Ok(());
    }

    // Apply, deepest edit first so earlier offsets stay valid.
    for (file, mut edits) in by_file {
        let src =
            std::fs::read_to_string(&file).map_err(|e| format!("could not read {file}: {e}"))?;
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        let mut out = src;
        for (start, end, repl) in edits {
            if start > end || end > out.len() {
                return Err(format!("internal error: stale edit offsets for {file}"));
            }
            out.replace_range(start..end, &repl);
        }
        std::fs::write(&file, &out).map_err(|e| format!("could not write {file}: {e}"))?;
    }

    // Prove the rewrite still checks — a Bytes id is a real change, so re-verify.
    let tree = load(path)?;
    let after = redstart_checker::check_diags(&tree);
    let errors = after.iter().filter(|d| d.is_error()).count();
    if errors > 0 {
        for d in after.iter().filter(|d| d.is_error()) {
            eprint!("{}", d.render());
        }
        return Err(format!(
            "rewrote {} entit{}, but the result no longer checks ({errors} error(s)) — please review",
            plan.rewrites.len(),
            if plan.rewrites.len() == 1 { "y" } else { "ies" }
        ));
    }

    let remaining = after.iter().filter(|d| d.code_short() == "W040").count();
    let note = if remaining > 0 {
        format!(" ({remaining} W040 site(s) remain — see the skipped entities above)")
    } else {
        String::new()
    };
    println!(
        "\n✓ rewrote {} entit{} to `Id<Bytes>`, still checks clean{note}",
        plan.rewrites.len(),
        if plan.rewrites.len() == 1 { "y" } else { "ies" }
    );
    Ok(())
}

/// Collect `.red` files: a single file as-is, or all under a directory
/// (skipping the `build` output directory and hidden folders).
fn collect_red_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut out = Vec::new();
    walk_red(path, &mut out).map_err(|e| format!("could not scan {}: {e}", path.display()))?;
    out.sort();
    Ok(out)
}

fn walk_red(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "build" || name.starts_with('.') {
                continue;
            }
            walk_red(&p, out)?;
        } else if p.extension().is_some_and(|e| e == "red") {
            out.push(p);
        }
    }
    Ok(())
}

fn cmd_new(name: &str) -> Result<(), String> {
    let root = PathBuf::from(name);
    if root.exists() {
        return Err(format!("`{name}` already exists"));
    }

    let src = root.join("src");
    let abis = src.join("abis");
    std::fs::create_dir_all(&abis)
        .map_err(|e| format!("could not create {}: {e}", abis.display()))?;

    write_file(
        &root.join("redstart.toml"),
        &format!("[project]\nname = \"{name}\"\nentry = \"src/main.red\"\nout_dir = \"build\"\n"),
    )?;
    write_file(
        &root.join(".gitignore"),
        "# Generated by `redstart build` — the `.red` files are the source of truth.\n\
         /build\n\
         /dist\n\
         node_modules/\n",
    )?;
    write_file(&src.join("main.red"), STARTER_MAIN)?;
    write_file(&src.join("accounts.red"), STARTER_ACCOUNTS)?;
    write_file(&abis.join("ERC20.json"), STARTER_ABI)?;

    println!("✓ created `{name}`");
    println!("  cd {name} && redstart build");
    Ok(())
}

fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("could not write {}: {e}", path.display()))
}

const STARTER_MAIN: &str = r#"// Welcome to Redstart. One source of truth, split across modules:
// this generates schema.graphql, subgraph.yaml, and the mappings together.
//
// Entities live in `accounts.red`, pulled in here with `mod`.

mod accounts;

abi ERC20 from "./abis/ERC20.json"

source Token {
  abi: ERC20
  network: mainnet
  address: 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
  startBlock: 6082465
}

handler on Token.Transfer(event) {
  let to = accounts::Account.loadOrCreate(event.params.to, { balance: BigInt.zero })
  to.balance = to.balance + event.params.value
  // auto-saved at handler end (dirty-tracked)
}
"#;

const STARTER_ACCOUNTS: &str = r#"// Entities for the starter subgraph, loaded via `mod accounts;` in main.red.

entity Account {
  id: Id<Bytes>
  balance: BigInt
  label: Option<String>   // Option<T> is how nullability is expressed — no `null`
}
"#;

const STARTER_ABI: &str = r#"[
  {
    "type": "event",
    "name": "Transfer",
    "inputs": [
      { "name": "from", "type": "address", "indexed": true },
      { "name": "to", "type": "address", "indexed": true },
      { "name": "value", "type": "uint256", "indexed": false }
    ]
  }
]
"#;
