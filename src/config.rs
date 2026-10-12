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
    pub theme: Theme,
    pub color: ColorMode,
    pub images: ImageMode,
    pub assets_dir: PathBuf,
    pub layout: LayoutMode,
    pub layout_explicit: bool,
    pub data_dir: PathBuf,
    pub storage_policy: crate::store::Policy,
}

#[derive(Debug)]
pub enum Command {
    Run(Config),
    Download(Config),
    Export { data_dir: PathBuf, output: PathBuf },
    Help,
    Version,
}

pub const HELP: &str = "Eevee — local-first evolution workbench

Usage: ev [OPTIONS]

Options:
  --theme dark|light    Terminal theme (default: dark)
  --color auto|256|mono Color mode (default: auto)
  --ascii              Use ASCII borders and luminance artwork
  --images auto|cells|off Artwork mode (default: auto; no automatic downloads)
  --assets-dir PATH    Local PNG directory (default: platform cache/eevee/artwork)
  --download-artwork   Explicitly download nine pinned PNGs and a source manifest
  --layout sanctuary|linear Layout (default: sanctuary)
  --data-dir PATH      SQLite store directory (default: platform data/eevee)
  --export-data PATH   Export readable stored tasks to a new JSON file; no reset
  --quota-kib N        Local SQLite quota (default: 65536; minimum: 64 KiB)
  --retention-days N   Archived purge eligibility age (default: 30; explicit purge only)
  -h, --help           Show this help
  -V, --version        Show the version

Auto color uses COLORTERM=truecolor/24bit, then TERM containing 256color,
otherwise monochrome. Nonempty NO_COLOR or TERM=dumb forces monochrome.
TERM=dumb or an explicitly non-UTF-8 locale also selects ASCII. Windows
defaults to UTF-8 without locale variables. Locale precedence: LC_ALL,
LC_CTYPE, LANG (first nonempty value). Repeated options use the last value.

Local durable tasks; no model execution. Storage failures retain editable drafts.
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
    let mut args = args.into_iter();
    let mut theme = Theme::Dark;
    let mut color = None;
    let mut ascii = false;
    let mut images = ImageMode::Auto;
    let mut assets_dir = default_assets_dir(&environment);
    let mut layout = LayoutMode::Sanctuary;
    let mut layout_explicit = false;
    let mut data_dir = default_data_dir(&environment);
    let mut export = None;
    let mut storage_policy = crate::store::Policy::default();
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
            "--download-artwork" if attached.is_none() => download = true,
            "--ascii" if attached.is_none() => ascii = true,
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
        theme,
        color,
        images,
        assets_dir,
        layout,
        layout_explicit,
        data_dir,
        storage_policy,
    };
    Ok(if download {
        Command::Download(config)
    } else {
        Command::Run(config)
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
            return PathBuf::from(cache).join("eevee").join("artwork");
        }
        if let Some(profile) = environment("USERPROFILE").filter(|value| !value.is_empty()) {
            return PathBuf::from(profile)
                .join("AppData")
                .join("Local")
                .join("eevee")
                .join("artwork");
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
                .join("artwork");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(cache) = environment("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(cache).join("eevee").join("artwork");
        }
        if let Some(home) = environment("HOME").filter(|value| !value.is_empty()) {
            let home = PathBuf::from(home);
            #[cfg(target_os = "macos")]
            return home
                .join("Library")
                .join("Caches")
                .join("eevee")
                .join("artwork");
            #[cfg(not(target_os = "macos"))]
            return home.join(".cache").join("eevee").join("artwork");
        }
    }
    PathBuf::from("artwork")
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
                .join("artwork")
        );
        assert_eq!(
            config(&[], &[("USERPROFILE", "C:\\Users\\Name")]).assets_dir,
            PathBuf::from("C:\\Users\\Name")
                .join("AppData")
                .join("Local")
                .join("eevee")
                .join("artwork")
        );
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
}
