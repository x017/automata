//! The `automata-cli` command line, shared by the `automata-cli`
//! binary and `oberon export`.
//!
//! ```text
//! automata-cli build [PROJECT] [--target linux|windows|all] [--template DIR]
//! automata-cli init [PROJECT] [--scene level.ron | --main main.py]
//! automata-cli check [PROJECT]
//! automata-cli stage [PROJECT] [--out DIR]
//! automata-cli templates path | list
//! automata-cli templates download [--target T]
//! automata-cli templates install ARCHIVE --target T
//! automata-cli templates dev PLAYER [--target T]
//! automata-cli templates pack DIR --target T --out ARCHIVE
//! ```

use std::path::{Path, PathBuf};

use crate::{BuildConfig, Level, Message, Target};

/// The usage text, as printed by `automata-cli help`.
pub const USAGE: &str = "\
automata-cli — the AUTOMATA command line: export games to Windows and Linux

usage:
  automata-cli build [PROJECT] [--target linux|windows|all] [--template DIR]
  automata-cli init [PROJECT] [--scene SCENE.ron | --main MAIN.py] [--name NAME]
  automata-cli check [PROJECT]
  automata-cli stage [PROJECT] [--out DIR]
  automata-cli templates path|list
  automata-cli templates download [--target linux|windows|all]
  automata-cli templates install ARCHIVE --target linux|windows
  automata-cli templates dev PLAYER [--target linux|windows]
  automata-cli templates pack DIR --target linux|windows --out ARCHIVE

PROJECT defaults to the current folder; settings come from PROJECT/build.ron.
`stage` writes only the game folder a build ships (game files and game.json),
by default to PROJECT/<output>/game; the Android build packs it into an APK.";

struct Args {
    positional: Vec<String>,
    flags: Vec<(String, String)>,
}

impl Args {
    fn parse(raw: impl Iterator<Item = String>) -> Self {
        let mut positional = Vec::new();
        let mut flags = Vec::new();
        let mut it = raw.peekable();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                if let Some((k, v)) = name.split_once('=') {
                    flags.push((k.to_string(), v.to_string()));
                } else {
                    let value = it.next_if(|n| !n.starts_with("--")).unwrap_or_default();
                    flags.push((name.to_string(), value));
                }
            } else {
                positional.push(a);
            }
        }
        Self { positional, flags }
    }

    fn flag(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn targets(&self, default: &[Target]) -> Result<Vec<Target>, String> {
        match self.flag("target") {
            None => Ok(default.to_vec()),
            Some("all") => Ok(Target::ALL.to_vec()),
            Some(t) => Target::parse(t)
                .map(|t| vec![t])
                .ok_or_else(|| format!("unknown target: {t}")),
        }
    }
}

fn print(m: &Message) {
    match m.level {
        Level::Info => println!("  {}", m.text),
        Level::Warning => eprintln!("warning: {}", m.text),
        Level::Error => eprintln!("error: {}", m.text),
    }
}

fn project(args: &Args, index: usize) -> PathBuf {
    args.positional
        .get(index)
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn absolute(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).map_or_else(|_| p.to_path_buf(), crate::plain_path)
}

/// [`USAGE`] with `program` (`"oberon export"`, say) in place of
/// `automata-cli`.
#[must_use]
pub fn usage(program: &str) -> String {
    USAGE.replace("automata-cli", program)
}

/// Runs a command line (without the program name): `build`, `init`,
/// `check`, `templates …` or `help`. `program` names the command in
/// messages.
///
/// # Errors
///
/// Returns the message to print when the command fails.
pub fn run(args: impl IntoIterator<Item = String>, program: &str) -> Result<(), String> {
    let args = Args::parse(args.into_iter());
    let Some(cmd) = args.positional.first().cloned() else {
        println!("{}", usage(program));
        return Ok(());
    };
    match cmd.as_str() {
        "build" => {
            let project = absolute(&project(&args, 1));
            let cfg = BuildConfig::for_project(&project, None);
            let targets = args.targets(&cfg.targets)?;
            let mut failed = 0;
            for target in targets {
                let template = match args.flag("template") {
                    Some(dir) => PathBuf::from(dir),
                    None => crate::find_template(target).ok_or_else(|| {
                        format!(
                            "no {} template installed in {} (run: {program} templates download --target {})",
                            target.label(),
                            crate::templates_dir().display(),
                            target.name()
                        )
                    })?,
                };
                println!("{} → {}", cfg.name, target.label());
                match crate::export(&project, &cfg, target, &template, &mut |m| print(&m)) {
                    Ok(out) => println!("built {}", out.executable.display()),
                    Err(e) => {
                        eprintln!("error: {e}");
                        failed += 1;
                    }
                }
            }
            if failed > 0 {
                return Err(format!("{failed} build(s) failed"));
            }
        }
        "init" => {
            let project = absolute(&project(&args, 1));
            let scene = args.flag("scene").map(|s| project.join(s));
            let mut cfg = BuildConfig::for_project(&project, scene.as_deref());
            if let Some(main) = args.flag("main") {
                cfg.main_script = Some(main.to_string());
                cfg.start_scene = None;
            }
            if let Some(name) = args.flag("name") {
                cfg.name = name.to_string();
            }
            cfg.save(&project)?;
            println!("wrote {}", project.join(crate::CONFIG_FILE).display());
        }
        "check" => {
            let project = absolute(&project(&args, 1));
            let cfg = BuildConfig::for_project(&project, None);
            let problems = crate::validate(&project, &cfg);
            problems.iter().for_each(print);
            if problems.iter().any(|m| m.level == Level::Error) {
                return Err("the project has errors".into());
            }
            println!("{} is ready to export", cfg.name);
        }
        "stage" => {
            let project = absolute(&project(&args, 1));
            let cfg = BuildConfig::for_project(&project, None);
            let out = args.flag("out").map_or_else(
                || project.join(&cfg.output).join("game"),
                |dir| std::path::absolute(dir).unwrap_or_else(|_| PathBuf::from(dir)),
            );
            let n = crate::stage_game(&project, &cfg, &out, &mut |m| print(&m))?;
            println!("staged {} ({n} files) in {}", cfg.name, out.display());
        }
        "templates" => templates(&args, program)?,
        "help" | "-h" | "--help" => println!("{}", usage(program)),
        other => return Err(format!("unknown command: {other}\n\n{}", usage(program))),
    }
    Ok(())
}

fn templates(args: &Args, program: &str) -> Result<(), String> {
    let sub = args.positional.get(1).map_or("list", String::as_str);
    let host = Target::host().map_or_else(Vec::new, |t| vec![t]);
    match sub {
        "path" => println!("{}", crate::templates_dir().display()),
        "list" => {
            println!("templates in {}", crate::templates_dir().display());
            for t in Target::ALL {
                match crate::find_template(t) {
                    Some(dir) if crate::template_has_runtime(&dir) => {
                        println!("  {:<8} installed", t.name());
                    }
                    Some(_) => println!(
                        "  {:<8} installed (development: uses the Python on PATH)",
                        t.name()
                    ),
                    None => println!("  {:<8} missing", t.name()),
                }
            }
        }
        "download" => {
            for t in args.targets(&Target::ALL)? {
                println!("downloading the {} template…", t.label());
                let dir = crate::download_template(t)?;
                println!("installed {}", dir.display());
            }
        }
        "install" => {
            let archive = args
                .positional
                .get(2)
                .ok_or("templates install ARCHIVE --target T")?;
            for t in args.targets(&host)? {
                let dir = crate::install_template(Path::new(archive), t)?;
                println!("installed {}", dir.display());
            }
        }
        "dev" => {
            let player = args.positional.get(2).ok_or("templates dev PLAYER")?;
            for t in args.targets(&host)? {
                let dir = crate::install_dev_template(Path::new(player), t)?;
                println!("installed a development template in {}", dir.display());
            }
        }
        "pack" => {
            let dir = args
                .positional
                .get(2)
                .ok_or("templates pack DIR --target T --out ARCHIVE")?;
            let target = args
                .targets(&host)?
                .into_iter()
                .next()
                .ok_or("--target is required")?;
            let out = args.flag("out").ok_or("--out is required")?;
            crate::pack_template(Path::new(dir), target, Path::new(out))?;
            println!("packed {out}");
        }
        other => {
            return Err(format!(
                "unknown templates command: {other} (see `{program} help`)"
            ))
        }
    }
    Ok(())
}
