use std::borrow::Cow;
use std::env;
use std::ffi::OsString;

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

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub ascii: bool,
    pub theme: Theme,
    pub color: ColorMode,
}

#[derive(Debug)]
pub enum Command {
    Run(Config),
    Help,
    Version,
}

pub const HELP: &str = "Eevee — local-first evolution workbench

Usage: eevee [OPTIONS]

Options:
  --theme dark|light    Terminal theme (default: dark)
  --color auto|256|mono Color mode (default: auto)
  --ascii              Use ASCII borders and companion motifs
  -h, --help           Show this help
  -V, --version        Show the version

Auto color uses COLORTERM=truecolor/24bit, then TERM containing 256color,
otherwise monochrome. Nonempty NO_COLOR or TERM=dumb forces monochrome.
TERM=dumb or a locale without UTF-8 also selects ASCII. Locale precedence:
LC_ALL, LC_CTYPE, LANG (first nonempty value). Repeated options use the last value.

Phase 1 is a session-local shell: no model execution or task persistence.
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
            "--ascii" if attached.is_none() => ascii = true,
            "--help" | "-h" if attached.is_none() => action = Some(Command::Help),
            "--version" | "-V" if attached.is_none() => action = Some(Command::Version),
            _ => return Err(format!("unrecognized argument {text:?}; see --help")),
        }
    }
    if let Some(action) = action {
        return Ok(action);
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
    let utf8 = locale
        .as_deref()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            let encoding = value.split('@').next().unwrap_or(value);
            let encoding = encoding.rsplit('.').next().unwrap_or(encoding);
            encoding.eq_ignore_ascii_case("utf-8") || encoding.eq_ignore_ascii_case("utf8")
        });
    Ok(Command::Run(Config {
        ascii: ascii || dumb || !utf8,
        theme,
        color,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], variables: &[(&str, &str)]) -> Result<Command, String> {
        parse_with(args.iter().map(OsString::from), |key| {
            variables
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| OsString::from(value))
        })
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

    #[cfg(unix)]
    #[test]
    fn non_unicode_arguments_are_usage_errors() {
        use std::os::unix::ffi::OsStringExt;
        let invalid = OsString::from_vec(vec![0xff]);
        assert!(parse_with([invalid.clone()], |_| None).is_err());
        assert!(parse_with([OsString::from("--theme"), invalid], |_| None).is_err());
    }
}
