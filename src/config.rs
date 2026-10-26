use std::borrow::Cow;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    TrueColor,
    Indexed,
    Mono,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMode {
    Auto,
    Cells,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    Sanctuary,
    Linear,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub ascii: bool,
    pub no_animation: bool,
    pub animation_override: Option<bool>,
    pub theme: Theme,
    pub color: ColorMode,
    pub images: ImageMode,
    pub assets_dir: PathBuf,
    pub layout: LayoutMode,
    pub layout_explicit: bool,
    pub data_dir: PathBuf,
    pub storage_policy: crate::store::Policy,
    pub context_exclusions: Vec<PathBuf>,
    pub capture_capacity: u64,
}

#[derive(Debug)]
pub enum Command {
    Run(Config),
    Download(Config),
    Export {
        data_dir: PathBuf,
        output: PathBuf,
    },
    Snapshot {
        operation: SnapshotOperation,
        source: PathBuf,
        destination: Option<PathBuf>,
    },
    Update {
        check_only: bool,
        no_animation: bool,
    },
    Uninstall,
    Help,
    Version,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotOperation {
    Create,
    Verify,
    Restore,
}

pub const HELP: &str = "Eevee — local-first evolution workbench

Usage: ev [OPTIONS]
       ev update [--check] [--no-animation]
       ev uninstall
       ev snapshot create --data-dir SOURCE --destination NEW --acknowledge-private-data
       ev snapshot verify --source BACKUP
       ev snapshot restore --source BACKUP --destination NEW --acknowledge-private-data

Binary lifecycle:
  update --check       Check the latest release without changing files
  update               Update an installer-owned binary in its original directory
  uninstall            Remove the owned binary and licenses; keep all user data

Offline data recovery (never starts the application):
  snapshot create      Complete versioned-manifest backup; requires stopped writers
  snapshot verify      Read-only member/digest integrity inspection, not reader admission
  snapshot restore     Validate with this reader and restore into a nonexisting directory
  Snapshot format 1 includes origin, private tasks/drafts/journals/outputs/artifacts,
  archived tasks, preferences and ANY saved app secrets/credentials. Not encrypted
  or redacted; no external credential-store access. Explicit privacy acknowledgement
  is required before copying. No source migration/repair, merge or overwrite.
  Failures may retain marked incomplete copies, never usable restored app data.
  Restore retains project identities and historical unknowns, not active requests,
  provider sessions or usable grants. No automatic dispatch or execution.

Options:
  --theme dark|light    Terminal theme (default: dark)
  --color auto|256|mono Color mode (default: auto)
  --ascii              Use ASCII borders (cell previews use luminance)
  --no-animation       Force static activity and transitions for this launch
  --animation          Allow motion for this launch, overriding saved preference
  --images auto|cells|off Native pixels, low-res cell preview, or no image (default: auto)
  --assets-dir PATH    Local PNG directory (default: platform cache/eevee/sprites)
  --download-artwork   Download/resume nine pinned original sprites and manifest
  --layout sanctuary|linear Layout (default: sanctuary)
  --data-dir PATH      SQLite store directory (default: platform data/eevee)
  --export-data PATH   Export readable stored tasks to a new JSON file; no reset
  --quota-kib N        Local SQLite quota (default: 65536; minimum: 64 KiB)
  --retention-days N   Archived purge eligibility age (default: 30; explicit purge only)
  --exclude-context PATH Exclude a repository-relative prefix (repeatable, conservative case-insensitive)
  --spool-kib N        Per-command capture reservation (default: 64; range: 1..16384 KiB)
  -h, --help           Show this help
  -V, --version        Show the version

Auto color uses COLORTERM=truecolor/24bit, then TERM containing 256color,
otherwise monochrome. Nonempty NO_COLOR or TERM=dumb forces monochrome.
TERM=dumb or an explicitly non-UTF-8 locale also selects ASCII. Windows
defaults to UTF-8 without locale variables. Locale precedence: LC_ALL,
LC_CTYPE, LANG (first nonempty value). Repeated options use the last value.
The interface is English. Unicode requests, paths and source text remain supported.

Auto images require negotiated Kitty/Sixel support and real pixel/cell dimensions.
Unsupported or too-small terminals show text, not a pixelated approximation.
--images cells explicitly opts into low-resolution previews. Startup stays offline.

Type in the Request box; Enter saves and sends, Ctrl+S only saves locally.
Use / commands: /connect, /provider, /model, /effort, /forms, /images, /help.
Commands, writes and risky connections retain review; model pickers fetch metadata only.
Secret detection is best-effort. Purge is not forensic secure erasure.
";

pub fn parse() -> Result<Command, String> {
    parse_with(env::args_os().skip(1), |key| env::var_os(key))
}

fn value<'a>(
    flag: &str,
    attached: Option<&'a str>,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<Cow<'a, str>, String> {
    if let Some(value) = attached {
        return Ok(Cow::Borrowed(value));
    }
    let value = args
        .next()
        .ok_or_else(|| format!("{flag} requires a value; see --help"))?;
    value
        .into_string()
        .map(Cow::Owned)
        .map_err(|value| format!("non-Unicode value for {flag}: {value:?}"))
}

fn parse_with(
    args: impl IntoIterator<Item = OsString>,
    environment: impl Fn(&str) -> Option<OsString>,
) -> Result<Command, String> {
    let mut args = args.into_iter().peekable();
    if args.peek().is_some_and(|argument| argument == "snapshot") {
        args.next();
        return parse_snapshot(args);
    }
    if let Some(command @ ("update" | "uninstall")) =
        args.peek().and_then(|argument| argument.to_str())
    {
        let update = command == "update";
        args.next();
        let option = args.next();
        if option
            .as_ref()
            .is_some_and(|value| value == "--help" || value == "-h")
            && args.peek().is_none()
        {
            return Ok(Command::Help);
        }
        let mut check_only = false;
        let mut no_animation = false;
        for option in option.into_iter().chain(args) {
            if update && option == "--check" && !check_only {
                check_only = true;
            } else if update && option == "--no-animation" && !no_animation {
                no_animation = true;
            } else {
                return Err(if update {
                    "usage: ev update [--check] [--no-animation]"
                } else {
                    "usage: ev uninstall"
                }
                .into());
            }
        }
        return Ok(if update {
            Command::Update {
                check_only,
                no_animation,
            }
        } else {
            Command::Uninstall
        });
    }
    let mut theme = Theme::Dark;
    let mut color = None;
    let mut ascii = false;
    let mut no_animation = false;
    let mut animation_override = None;
    let mut images = ImageMode::Auto;
    let mut assets_dir = default_assets_dir(&environment);
    let mut layout = LayoutMode::Sanctuary;
    let mut layout_explicit = false;
    let mut data_dir = default_data_dir(&environment);
    let mut export = None;
    let mut storage_policy = crate::store::Policy::default();
    let mut context_exclusions = Vec::new();
    let mut capture_capacity = 64 * 1024;
    let mut download = false;
    let mut action = None;
    while let Some(argument) = args.next() {
        let text = argument
            .to_str()
            .ok_or_else(|| format!("non-Unicode argument: {argument:?}"))?;
        let (flag, attached) = text
            .split_once('=')
            .map_or((text, None), |(flag, value)| (flag, Some(value)));
        match flag {
            "--theme" => {
                let value = value(flag, attached, &mut args)?;
                theme = match value.as_ref() {
                    "dark" => Theme::Dark,
                    "light" => Theme::Light,
                    _ => {
                        return Err(format!(
                            "invalid --theme value {value:?}; expected dark or light"
                        ));
                    }
                };
            }
            "--color" => {
                let value = value(flag, attached, &mut args)?;
                color = match value.as_ref() {
                    "auto" => None,
                    "256" => Some(ColorMode::Indexed),
                    "mono" => Some(ColorMode::Mono),
                    _ => {
                        return Err(format!(
                            "invalid --color value {value:?}; expected auto, 256, or mono"
                        ));
                    }
                };
            }
            "--images" => {
                let value = value(flag, attached, &mut args)?;
                images = match value.as_ref() {
                    "auto" => ImageMode::Auto,
                    "cells" => ImageMode::Cells,
                    "off" => ImageMode::Off,
                    _ => {
                        return Err(format!(
                            "invalid --images value {value:?}; expected auto, cells, or off"
                        ));
                    }
                };
            }
            "--layout" => {
                layout_explicit = true;
                let value = value(flag, attached, &mut args)?;
                layout = match value.as_ref() {
                    "sanctuary" => LayoutMode::Sanctuary,
                    "linear" => LayoutMode::Linear,
                    _ => {
                        return Err(format!(
                            "invalid --layout value {value:?}; expected sanctuary or linear"
                        ));
                    }
                };
            }
            "--assets-dir" => {
                assets_dir = if let Some(path) = attached {
                    PathBuf::from(path)
                } else {
                    PathBuf::from(
                        args.next()
                            .ok_or("--assets-dir requires a path; see --help")?,
                    )
                };
                if assets_dir.as_os_str().is_empty() {
                    return Err("--assets-dir requires a nonempty path".into());
                }
            }
            "--data-dir" | "--export-data" => {
                let path = if let Some(path) = attached {
                    PathBuf::from(path)
                } else {
                    PathBuf::from(
                        args.next()
                            .ok_or_else(|| format!("{flag} requires a path"))?,
                    )
                };
                if path.as_os_str().is_empty() {
                    return Err(format!("{flag} requires a nonempty path"));
                }
                if flag == "--data-dir" {
                    data_dir = Some(path);
                } else {
                    export = Some(path);
                }
            }
            "--quota-kib" => {
                let amount = value(flag, attached, &mut args)?
                    .parse::<u64>()
                    .map_err(|_| "--quota-kib requires an integer of at least 64")?;
                if amount < 64 {
                    return Err("--quota-kib requires at least 64 KiB".into());
                }
                storage_policy.quota_bytes =
                    amount.checked_mul(1024).ok_or("--quota-kib is too large")?;
            }
            "--retention-days" => {
                storage_policy.retention_days = value(flag, attached, &mut args)?
                    .parse::<u32>()
                    .map_err(|_| "--retention-days requires a nonnegative integer")?;
            }
            "--exclude-context" => {
                let path = PathBuf::from(&*value(flag, attached, &mut args)?);
                if context_exclusions.len() >= 32
                    || path.as_os_str().is_empty()
                    || !path
                        .components()
                        .all(|part| matches!(part, std::path::Component::Normal(_)))
                {
                    return Err(
                        "--exclude-context requires a relative path prefix; at most 32 exclusions"
                            .into(),
                    );
                }
                context_exclusions.push(path);
            }
            "--spool-kib" => {
                let amount = value(flag, attached, &mut args)?
                    .parse::<u64>()
                    .map_err(|_| "--spool-kib requires an integer in 1..16384")?;
                if !(1..=16384).contains(&amount) {
                    return Err("--spool-kib must be in 1..16384".into());
                }
                capture_capacity = amount * 1024;
            }
            "--download-artwork" if attached.is_none() => download = true,
            "--ascii" if attached.is_none() => ascii = true,
            "--no-animation" if attached.is_none() => {
                no_animation = true;
                animation_override = Some(false);
            }
            "--animation" if attached.is_none() => {
                no_animation = false;
                animation_override = Some(true);
            }
            "--help" | "-h" if attached.is_none() => action = Some(Command::Help),
            "--version" | "-V" if attached.is_none() => action = Some(Command::Version),
            _ => return Err(format!("unrecognized argument {text:?}; see --help")),
        }
    }
    if let Some(action) = action {
        return Ok(action);
    }
    let data_dir = data_dir.ok_or("platform data directory unavailable; use --data-dir PATH")?;
    if data_dir.to_str().is_none() {
        return Err(
            "--data-dir must be a Unicode path; non-Unicode identities are not merged".into(),
        );
    }
    if let Some(output) = export {
        if download {
            return Err("--export-data cannot be combined with --download-artwork".into());
        }
        return Ok(Command::Export { data_dir, output });
    }

    let term = environment("TERM");
    let term = term.as_deref().and_then(|value| value.to_str());
    let dumb = term == Some("dumb");
    let no_color = environment("NO_COLOR").is_some_and(|value| !value.is_empty());
    let color = if dumb || no_color {
        ColorMode::Mono
    } else if let Some(color) = color {
        color
    } else {
        let colorterm = environment("COLORTERM");
        match colorterm.as_deref().and_then(|value| value.to_str()) {
            Some(value)
                if value.eq_ignore_ascii_case("truecolor")
                    || value.eq_ignore_ascii_case("24bit") =>
            {
                ColorMode::TrueColor
            }
            _ if term.is_some_and(|value| value.contains("256color")) => ColorMode::Indexed,
            _ => ColorMode::Mono,
        }
    };
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .find_map(|key| environment(key).filter(|value| !value.is_empty()));
    let utf8 = locale.as_deref().map_or(cfg!(windows), |value| {
        value.to_str().is_some_and(|value| {
            let encoding = value.split('@').next().unwrap_or(value);
            let encoding = encoding.rsplit('.').next().unwrap_or(encoding);
            encoding.eq_ignore_ascii_case("utf-8") || encoding.eq_ignore_ascii_case("utf8")
        })
    });
    let config = Config {
        ascii: ascii || dumb || !utf8,
        no_animation: no_animation || dumb,
        animation_override: if dumb || !utf8 {
            Some(false)
        } else {
            animation_override
        },
        theme,
        color,
        images,
        assets_dir,
        layout,
        layout_explicit,
        data_dir,
        storage_policy,
        context_exclusions,
        capture_capacity,
    };
    Ok(if download {
        Command::Download(config)
    } else {
        Command::Run(config)
    })
}

fn parse_snapshot(mut args: impl Iterator<Item = OsString>) -> Result<Command, String> {
    let operation = match args.next().as_deref().and_then(|value| value.to_str()) {
        Some("create") => SnapshotOperation::Create,
        Some("verify") => SnapshotOperation::Verify,
        Some("restore") => SnapshotOperation::Restore,
        Some("--help" | "-h") => return Ok(Command::Help),
        _ => return Err("usage: ev snapshot create|verify|restore; see --help".into()),
    };
    let mut source = None;
    let mut destination = None;
    let mut acknowledged = false;
    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            return Ok(Command::Help);
        }
        if argument == "--acknowledge-private-data" {
            if acknowledged {
                return Err("Duplicate privacy acknowledgement".into());
            }
            acknowledged = true;
            continue;
        }
        let text = argument.to_str().ok_or("Snapshot option must be Unicode")?;
        let (flag, attached) = text
            .split_once('=')
            .map_or((text, None), |(a, b)| (a, Some(b)));
        let target = match flag {
            "--data-dir" if operation == SnapshotOperation::Create => &mut source,
            "--source" if operation != SnapshotOperation::Create => &mut source,
            "--destination" if operation != SnapshotOperation::Verify => &mut destination,
            _ => {
                return Err(format!(
                    "Unexpected snapshot option {argument:?}; see --help"
                ));
            }
        };
        if target.is_some() {
            return Err(format!("Duplicate snapshot option {flag}"));
        }
        let path = attached
            .map(OsString::from)
            .or_else(|| args.next())
            .ok_or_else(|| format!("{flag} requires a path"))?;
        if path.is_empty() {
            return Err(format!("{flag} requires a nonempty path"));
        }
        *target = Some(PathBuf::from(path));
    }
    if operation != SnapshotOperation::Verify && !acknowledged {
        return Err(format!(
            "{}\\nCopy refused: add --acknowledge-private-data only after reviewing these effects.",
            crate::store::snapshot::DISCLOSURE
        ));
    }
    if operation == SnapshotOperation::Verify && acknowledged {
        return Err("Verification copies no data; omit --acknowledge-private-data".into());
    }
    let source = source
        .ok_or("Snapshot requires an explicit --data-dir (create) or --source (verify/restore)")?;
    if operation != SnapshotOperation::Verify && destination.is_none() {
        return Err("Snapshot copy requires --destination NEW (must not exist)".into());
    }
    Ok(Command::Snapshot {
        operation,
        source,
        destination,
    })
}

fn default_data_dir(environment: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    #[cfg(windows)]
    let base = environment("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        environment("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = environment("XDG_DATA_HOME")
        .filter(|path| PathBuf::from(path).is_absolute())
        .map(PathBuf::from)
        .or_else(|| environment("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    base.filter(|path| path.is_absolute())
        .map(|path| path.join("eevee"))
}

fn default_assets_dir(environment: &impl Fn(&str) -> Option<OsString>) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(cache) = environment("LOCALAPPDATA").filter(|value| !value.is_empty()) {
            return PathBuf::from(cache).join("eevee").join("sprites");
        }
        if let Some(profile) = environment("USERPROFILE").filter(|value| !value.is_empty()) {
            return PathBuf::from(profile)
                .join("AppData")
                .join("Local")
                .join("eevee")
                .join("sprites");
        }
        if let (Some(drive), Some(path)) = (
            environment("HOMEDRIVE").filter(|value| !value.is_empty()),
            environment("HOMEPATH").filter(|value| !value.is_empty()),
        ) {
            let mut home = drive;
            home.push(path);
            return PathBuf::from(home)
                .join("AppData")
                .join("Local")
                .join("eevee")
                .join("sprites");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(cache) = environment("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(cache).join("eevee").join("sprites");
        }
        if let Some(home) = environment("HOME").filter(|value| !value.is_empty()) {
            let home = PathBuf::from(home);
            #[cfg(target_os = "macos")]
            return home
                .join("Library")
                .join("Caches")
                .join("eevee")
                .join("sprites");
            #[cfg(not(target_os = "macos"))]
            return home.join(".cache").join("eevee").join("sprites");
        }
    }
    PathBuf::from("sprites")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], variables: &[(&str, &str)]) -> Result<Command, String> {
        parse_with(
            ["--data-dir", "isolated-test-data"]
                .iter()
                .chain(args.iter())
                .map(OsString::from),
            |key| {
                variables
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| OsString::from(value))
            },
        )
    }

    fn config(args: &[&str], variables: &[(&str, &str)]) -> Config {
        match parse(args, variables).unwrap() {
            Command::Run(config) => config,
            _ => panic!("expected runnable configuration"),
        }
    }

    #[test]
    fn color_capabilities_and_environment_overrides() {
        assert_eq!(
            config(&[], &[("COLORTERM", "truecolor")]).color,
            ColorMode::TrueColor
        );
        assert_eq!(
            config(&[], &[("COLORTERM", "24bit")]).color,
            ColorMode::TrueColor
        );
        assert_eq!(
            config(&[], &[("TERM", "xterm-256color")]).color,
            ColorMode::Indexed
        );
        assert_eq!(config(&[], &[("TERM", "xterm")]).color, ColorMode::Mono);
        assert_eq!(
            config(&["--color=256"], &[("NO_COLOR", "1")]).color,
            ColorMode::Mono
        );
        assert_eq!(
            config(&["--color=256"], &[("NO_COLOR", "")]).color,
            ColorMode::Indexed
        );
        let dumb = config(&["--color=256"], &[("TERM", "dumb"), ("LANG", "C.UTF-8")]);
        assert_eq!(dumb.color, ColorMode::Mono);
        assert!(dumb.ascii);
        assert_eq!(
            config(&["--color=mono"], &[("COLORTERM", "truecolor")]).color,
            ColorMode::Mono
        );
        assert_eq!(
            config(&["--color=256", "--color=auto"], &[]).color,
            ColorMode::Mono
        );
    }

    #[test]
    fn locale_precedence_is_first_nonempty_value() {
        assert!(!config(&[], &[("LANG", "ja_JP.UTF-8")]).ascii);
        assert!(!config(&[], &[("LC_CTYPE", "C.utf8@modifier"), ("LANG", "C")]).ascii);
        assert!(config(&[], &[("LC_ALL", "C"), ("LC_CTYPE", "C.UTF-8")]).ascii);
        assert!(!config(&[], &[("LC_ALL", ""), ("LANG", "C.UTF-8")]).ascii);
        assert!(config(&[], &[("LC_CTYPE", "C"), ("LANG", "C.UTF-8")]).ascii);
    }

    #[test]
    fn informational_flags_and_invalid_arguments() {
        assert!(matches!(parse(&["-h"], &[]), Ok(Command::Help)));
        assert!(matches!(parse(&["--help"], &[]), Ok(Command::Help)));
        assert!(matches!(parse(&["-V"], &[]), Ok(Command::Version)));
        assert!(matches!(parse(&["--version"], &[]), Ok(Command::Version)));
        for args in [
            vec!["--theme"],
            vec!["--color"],
            vec!["--theme=blue"],
            vec!["--color=truecolor"],
            vec!["--ascii=yes"],
            vec!["--unknown"],
            vec!["repository"],
            vec!["--help", "--unknown"],
        ] {
            assert!(parse(&args, &[]).is_err(), "{args:?}");
        }
    }

    #[test]
    fn binary_lifecycle_commands_reject_ambiguous_or_tui_arguments() {
        let lifecycle = |args: &[&str]| parse_with(args.iter().map(OsString::from), |_| None);
        assert!(matches!(
            lifecycle(&["update", "--check"]),
            Ok(Command::Update {
                check_only: true,
                no_animation: false
            })
        ));
        assert!(matches!(
            lifecycle(&["update"]),
            Ok(Command::Update {
                check_only: false,
                no_animation: false
            })
        ));
        assert!(matches!(lifecycle(&["uninstall"]), Ok(Command::Uninstall)));
        let invalid: &[&[&str]] = &[
            &["uninstall", "--check"],
            &["update", "--check", "--check"],
            &["update", "--data-dir", "/tmp/tasks"],
            &["uninstall", "--data-dir", "/tmp/tasks"],
            &["--images", "off", "uninstall"],
            &["update", "--help", "uninstall"],
        ];
        for args in invalid {
            assert!(lifecycle(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn artwork_and_layout_cli_boundaries() {
        let config = config(
            &[
                "--assets-dir",
                "portraits with spaces",
                "--images=cells",
                "--layout=linear",
            ],
            &[],
        );
        assert_eq!(config.assets_dir, PathBuf::from("portraits with spaces"));
        assert_eq!(config.images, ImageMode::Cells);
        assert_eq!(config.layout, LayoutMode::Linear);
        assert!(matches!(
            parse(&["--download-artwork"], &[]),
            Ok(Command::Download(_))
        ));
        for args in [
            vec!["--images"],
            vec!["--images=pixels"],
            vec!["--layout=grid"],
            vec!["--assets-dir"],
            vec!["--assets-dir="],
            vec!["--download-artwork=yes"],
        ] {
            assert!(parse(&args, &[]).is_err(), "{args:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_defaults_and_cache_fallbacks() {
        assert!(!config(&[], &[]).ascii);
        assert!(config(&[], &[("LANG", "C")]).ascii);
        assert_eq!(
            config(&[], &[("LOCALAPPDATA", "C:\\Users\\Name\\AppData\\Local")]).assets_dir,
            PathBuf::from("C:\\Users\\Name\\AppData\\Local")
                .join("eevee")
                .join("sprites")
        );
        assert_eq!(
            config(&[], &[("USERPROFILE", "C:\\Users\\Name")]).assets_dir,
            PathBuf::from("C:\\Users\\Name")
                .join("AppData")
                .join("Local")
                .join("eevee")
                .join("sprites")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn installer_and_startup_share_the_sprite_cache() {
        let environment = |key: &str| match key {
            "XDG_CACHE_HOME" => Some(OsString::from("/cache")),
            "HOME" => Some(OsString::from("/home/example")),
            _ => None,
        };
        let run = parse_with([OsString::from("--data-dir=/data")], environment).unwrap();
        let download = parse_with(
            [
                OsString::from("--download-artwork"),
                OsString::from("--data-dir=/data"),
            ],
            environment,
        )
        .unwrap();
        let (Command::Run(run), Command::Download(download)) = (run, download) else {
            panic!("expected runtime and installer download configurations");
        };
        assert_eq!(run.assets_dir, PathBuf::from("/cache/eevee/sprites"));
        assert_eq!(run.assets_dir, download.assets_dir);
    }

    #[cfg(unix)]
    #[test]
    fn separate_asset_path_preserves_non_unicode_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![b'a', b' ', 0xff]);
        let command = parse_with(
            [
                OsString::from("--assets-dir"),
                path.clone(),
                OsString::from("--data-dir"),
                OsString::from("isolated-test-data"),
            ],
            |_| None,
        )
        .unwrap();
        let Command::Run(config) = command else {
            panic!("expected run")
        };
        assert_eq!(config.assets_dir.into_os_string(), path);
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_arguments_are_usage_errors() {
        use std::os::unix::ffi::OsStringExt;
        let invalid = OsString::from_vec(vec![0xff]);
        assert!(parse_with([invalid.clone()], |_| None).is_err());
        assert!(parse_with([OsString::from("--theme"), invalid], |_| None).is_err());
    }
    #[test]
    fn motion_cli_is_last_wins_and_independent_of_color() {
        let utf8 = &[("LANG", "en_US.UTF-8")];
        assert_eq!(
            config(
                &["--animation"],
                &[("LANG", "en_US.UTF-8"), ("NO_COLOR", "1")]
            )
            .animation_override,
            Some(true)
        );
        assert_eq!(
            config(&["--no-animation", "--animation"], utf8).animation_override,
            Some(true)
        );
        assert_eq!(
            config(&["--animation", "--no-animation"], utf8).animation_override,
            Some(false)
        );
        assert_eq!(
            config(&["--animation"], &[("TERM", "dumb")]).animation_override,
            Some(false)
        );
        assert!(matches!(
            parse_with(
                ["update", "--no-animation", "--check"].map(OsString::from),
                |_| None
            ),
            Ok(Command::Update {
                check_only: true,
                no_animation: true
            })
        ));
    }
    #[test]
    fn snapshot_root_actions_are_explicit_offline_and_privacy_acknowledged() {
        assert!(matches!(
            parse_with(
                ["snapshot", "verify", "--source", "backup"].map(OsString::from),
                |_| None
            ),
            Ok(Command::Snapshot {
                operation: SnapshotOperation::Verify,
                destination: None,
                ..
            })
        ));
        for operation in ["create", "restore"] {
            let source_flag = if operation == "create" {
                "--data-dir"
            } else {
                "--source"
            };
            let arguments = [
                "snapshot",
                operation,
                source_flag,
                "source",
                "--destination",
                "new",
            ];
            let error = parse_with(arguments.map(OsString::from), |_| None).unwrap_err();
            assert!(error.contains("NOT encrypted"));
            assert!(error.contains("--acknowledge-private-data"));
            assert!(matches!(
                parse_with(
                    arguments
                        .into_iter()
                        .chain(["--acknowledge-private-data"])
                        .map(OsString::from),
                    |_| None
                ),
                Ok(Command::Snapshot {
                    destination: Some(_),
                    ..
                })
            ));
        }
        for arguments in [
            vec![
                "snapshot",
                "restore",
                "--source",
                "backup",
                "--acknowledge-private-data",
            ],
            vec![
                "snapshot",
                "verify",
                "--source",
                "backup",
                "--download-artwork",
            ],
            vec![
                "snapshot", "verify", "--source", "backup", "--source", "other",
            ],
            vec![
                "snapshot",
                "create",
                "--source",
                "backup",
                "--acknowledge-private-data",
            ],
        ] {
            assert!(parse_with(arguments.into_iter().map(OsString::from), |_| None).is_err());
        }
        assert!(HELP.contains("ANY saved app secrets"));
        assert!(HELP.contains("No source migration"));
    }
}
