//! Local cost *estimates* from provider-reported usage and a dated price table.
//! Never billing: counters stay raw in `usage`; nothing here is written back.
use crate::{providers::Agent, workspace::usage::Record};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Date the compiled-in prices were last checked. Older than 90 days is stale.
pub const BUILT_IN_AS_OF: &str = "2025-11-01";
const STALE_DAYS: i64 = 90;
const FILE_LIMIT: u64 = 64 * 1024;
pub const FILE_NAME: &str = "usage.toml";

/// USD per one million tokens.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub provider: String,
    /// Exact model or prefix; the longest matching prefix wins.
    pub model: String,
    pub input: f64,
    #[serde(default)]
    pub cached_input: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    pub output: f64,
    /// Above this many input tokens per request, `tier_input`/`tier_output` apply.
    #[serde(default)]
    pub above_tokens: Option<u64>,
    #[serde(default)]
    pub tier_input: Option<f64>,
    #[serde(default)]
    pub tier_output: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    BuiltIn,
    User,
}

macro_rules! price {
    ($provider:literal, $model:literal, $input:expr, $cached:expr, $write:expr, $output:expr) => {
        Price {
            provider: String::new() + $provider,
            model: String::new() + $model,
            input: $input,
            cached_input: $cached,
            cache_write: $write,
            output: $output,
            above_tokens: None,
            tier_input: None,
            tier_output: None,
        }
    };
}

/// Public list prices as of [`BUILT_IN_AS_OF`]. Unlisted models show `no price`.
fn built_in() -> Vec<Price> {
    let mut rows = vec![
        price!("openai", "gpt-5", 1.25, Some(0.125), None, 10.0),
        price!("openai", "gpt-5-mini", 0.25, Some(0.025), None, 2.0),
        price!("openai", "gpt-5-nano", 0.05, Some(0.005), None, 0.4),
        price!("openai", "gpt-5-pro", 15.0, None, None, 120.0),
        price!("openai", "gpt-4.1", 2.0, Some(0.5), None, 8.0),
        price!("openai", "gpt-4.1-mini", 0.4, Some(0.1), None, 1.6),
        price!("openai", "gpt-4.1-nano", 0.1, Some(0.025), None, 0.4),
        price!("openai", "gpt-4o", 2.5, Some(1.25), None, 10.0),
        price!("openai", "gpt-4o-mini", 0.15, Some(0.075), None, 0.6),
        price!("openai", "o3", 2.0, Some(0.5), None, 8.0),
        price!("openai", "o3-mini", 1.1, Some(0.55), None, 4.4),
        price!("openai", "o3-pro", 20.0, None, None, 80.0),
        price!("openai", "o4-mini", 1.1, Some(0.275), None, 4.4),
        price!(
            "anthropic",
            "claude-opus-4-1",
            15.0,
            Some(1.5),
            Some(18.75),
            75.0
        ),
        price!(
            "anthropic",
            "claude-opus-4-0",
            15.0,
            Some(1.5),
            Some(18.75),
            75.0
        ),
        price!(
            "anthropic",
            "claude-opus-4-2025",
            15.0,
            Some(1.5),
            Some(18.75),
            75.0
        ),
        price!(
            "anthropic",
            "claude-opus-4-5",
            5.0,
            Some(0.5),
            Some(6.25),
            25.0
        ),
        price!(
            "anthropic",
            "claude-sonnet-4",
            3.0,
            Some(0.3),
            Some(3.75),
            15.0
        ),
        price!(
            "anthropic",
            "claude-3-7-sonnet",
            3.0,
            Some(0.3),
            Some(3.75),
            15.0
        ),
        price!(
            "anthropic",
            "claude-haiku-4-5",
            1.0,
            Some(0.1),
            Some(1.25),
            5.0
        ),
        price!(
            "anthropic",
            "claude-3-5-haiku",
            0.8,
            Some(0.08),
            Some(1.0),
            4.0
        ),
        price!("google", "gemini-2.5-flash", 0.3, Some(0.075), None, 2.5),
        price!(
            "google",
            "gemini-2.5-flash-lite",
            0.1,
            Some(0.025),
            None,
            0.4
        ),
    ];
    let mut pro = price!("google", "gemini-2.5-pro", 1.25, Some(0.31), None, 10.0);
    pro.above_tokens = Some(200_000);
    pro.tier_input = Some(2.5);
    pro.tier_output = Some(15.0);
    rows.push(pro);
    rows
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    meter: Option<bool>,
    #[serde(default)]
    budget_usd: Option<f64>,
    #[serde(default)]
    price: Vec<Price>,
}

/// Meter preferences and user price rows from `<data dir>/usage.toml`.
#[derive(Clone, Debug)]
pub struct Table {
    pub path: PathBuf,
    pub meter: bool,
    pub budget_usd: Option<f64>,
    user: Vec<Price>,
    built_in: Vec<Price>,
    pub error: Option<String>,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            path: PathBuf::from(FILE_NAME),
            meter: true,
            budget_usd: None,
            user: Vec::new(),
            built_in: built_in(),
            error: None,
        }
    }
}

impl Table {
    /// A missing file is the default; an invalid one keeps defaults and says why.
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(FILE_NAME);
        let mut table = Self {
            path: path.clone(),
            ..Self::default()
        };
        match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return table,
            Err(_) => {
                table.error = Some(format!("Cannot read {}", path.display()));
                return table;
            }
            Ok(meta) if !meta.is_file() || meta.len() > FILE_LIMIT => {
                table.error = Some(format!(
                    "{} must be a regular file of at most 64 KiB",
                    path.display()
                ));
                return table;
            }
            Ok(_) => {}
        }
        match std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| Self::parse(&text))
        {
            Ok(file) => {
                table.meter = file.meter.unwrap_or(true);
                table.budget_usd = file.budget_usd;
                table.user = file.price;
            }
            Err(error) => table.error = Some(format!("{}: {error}", path.display())),
        }
        table
    }

    fn parse(text: &str) -> Result<File, String> {
        let file: File = toml::from_str(text).map_err(|error| error.message().to_owned())?;
        let valid = |value: f64| value.is_finite() && (0.0..=10_000.0).contains(&value);
        for row in &file.price {
            let rates = [row.input, row.output]
                .into_iter()
                .chain(row.cached_input)
                .chain(row.cache_write)
                .chain(row.tier_input)
                .chain(row.tier_output);
            if row.model.is_empty() || row.provider.is_empty() || !rates.into_iter().all(valid) {
                return Err(format!("invalid price row for {:?}", row.model));
            }
        }
        if file.budget_usd.is_some_and(|budget| !valid(budget)) {
            return Err("budget_usd must be between 0 and 10000".into());
        }
        Ok(file)
    }

    /// Writes only the meter preferences, keeping every user price row.
    pub fn save_preferences(&self) -> Result<(), String> {
        let mut text = String::from(
            "# Eevee usage meter. Prices are USD per 1M tokens; estimates, never billing.\n",
        );
        text.push_str(&format!("meter = {}\n", self.meter));
        if let Some(budget) = self.budget_usd {
            text.push_str(&format!("budget_usd = {budget}\n"));
        }
        for row in &self.user {
            text.push_str(&format!(
                "\n[[price]]\nprovider = {:?}\nmodel = {:?}\ninput = {}\noutput = {}\n",
                row.provider, row.model, row.input, row.output
            ));
            for (name, value) in [
                ("cached_input", row.cached_input),
                ("cache_write", row.cache_write),
                ("tier_input", row.tier_input),
                ("tier_output", row.tier_output),
            ] {
                if let Some(value) = value {
                    text.push_str(&format!("{name} = {value}\n"));
                }
            }
            if let Some(above) = row.above_tokens {
                text.push_str(&format!("above_tokens = {above}\n"));
            }
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&self.path, text).map_err(|error| error.to_string())
    }

    pub fn lookup(&self, provider: &str, model: &str) -> Option<(&Price, Source)> {
        fn best<'a>(rows: &'a [Price], provider: &str, model: &str) -> Option<&'a Price> {
            rows.iter()
                .filter(|row| {
                    row.provider.eq_ignore_ascii_case(provider) && model.starts_with(&row.model)
                })
                .max_by_key(|row| row.model.len())
        }
        best(&self.user, provider, model)
            .map(|row| (row, Source::User))
            .or_else(|| best(&self.built_in, provider, model).map(|row| (row, Source::BuiltIn)))
    }

    pub fn user_rows(&self) -> usize {
        self.user.len()
    }
}

/// Days from 1970-01-01 for a `YYYY-MM-DD` date (proleptic Gregorian).
fn days(date: &str) -> Option<i64> {
    let mut parts = date.split('-').map(|part| part.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Whether the compiled-in prices are older than 90 days at `now` (Unix seconds).
pub fn built_in_stale(now: u64) -> bool {
    days(BUILT_IN_AS_OF).is_some_and(|checked| (now / 86_400) as i64 - checked > STALE_DAYS)
}

/// How a route is billed, as far as Eevee can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Token × price on a direct API key.
    Api,
    /// Account/subscription login (ChatGPT): no per-token price.
    Plan,
    /// External agent program: its billing is its own.
    Agent,
    /// Loopback endpoint: local inference, no provider cost.
    Local,
}

pub fn route(agent: Agent, loopback: bool) -> Route {
    match agent {
        Agent::OpenAI | Agent::Anthropic | Agent::Google => Route::Api,
        Agent::Compatible if loopback => Route::Local,
        Agent::Compatible => Route::Api,
        Agent::ChatGPT => Route::Plan,
        Agent::Codex | Agent::Claude | Agent::OpenCode | Agent::Gemini | Agent::Devin => {
            Route::Agent
        }
    }
}

pub fn provider_key(agent: Agent) -> &'static str {
    match agent {
        Agent::OpenAI | Agent::ChatGPT | Agent::Codex => "openai",
        Agent::Anthropic | Agent::Claude => "anthropic",
        Agent::Google | Agent::Gemini => "google",
        Agent::Compatible => "compatible",
        Agent::OpenCode => "opencode",
        Agent::Devin => "devin",
    }
}

/// Token totals with explicit unknowns; never a fabricated zero.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    /// All input tokens, including cached reads/writes when known to be separate.
    pub input: u64,
    pub cached: u64,
    pub cache_write: u64,
    /// All output tokens, including reasoning.
    pub output: u64,
    pub reasoning: u64,
    pub requests: usize,
    /// Requests whose provider reported no attributable usage.
    pub unreported: usize,
    /// Some reported part could not be attributed or its subset relation is unknown.
    pub uncertain: bool,
    pub usd: f64,
    /// API requests with a complete estimate in `usd`.
    pub priced: usize,
    /// Requests whose cost could not be estimated (no price, unknown semantics).
    pub unpriced: usize,
    pub plan: usize,
    pub agent: usize,
    pub local: usize,
}

impl Totals {
    pub fn tokens(&self) -> u64 {
        self.input + self.output
    }
    pub fn add(&mut self, other: &Self) {
        self.input += other.input;
        self.cached += other.cached;
        self.cache_write += other.cache_write;
        self.output += other.output;
        self.reasoning += other.reasoning;
        self.requests += other.requests;
        self.unreported += other.unreported;
        self.uncertain |= other.uncertain;
        self.usd += other.usd;
        self.priced += other.priced;
        self.unpriced += other.unpriced;
        self.plan += other.plan;
        self.agent += other.agent;
        self.local += other.local;
    }
    /// Any part missing from the sum makes it a lower bound.
    pub fn lower_bound(&self) -> bool {
        self.unreported > 0 || self.uncertain
    }
}

/// One request's normalized tokens and estimate.
pub fn request(record: &Record, route: Route, table: &Table) -> Totals {
    let mut totals = Totals {
        requests: 1,
        ..Totals::default()
    };
    let mut reported = false;
    let mut cost = Some(0.0f64);
    for measurement in &record.measurements {
        let counts = measurement.attributable;
        if counts.is_unknown() {
            // Cumulative scope without baseline: known to exist, not attributable.
            if !measurement.report.counters.is_unknown() {
                totals.uncertain = true;
            }
            continue;
        }
        reported = true;
        let semantics = measurement.report.semantics;
        let raw_input = counts.input.unwrap_or(0);
        let cached = counts.cache_read.unwrap_or(0);
        let written = counts.cache_write.unwrap_or(0);
        let mut known = counts.input.is_some() && counts.output.is_some();
        // Fold cache tokens into the input total exactly once.
        let (input, fresh) = match (counts.cache_read, semantics.cache_read_in_input) {
            (None, _) | (Some(0), _) => (raw_input, raw_input),
            (Some(_), Some(true)) => (raw_input, raw_input.saturating_sub(cached)),
            (Some(_), Some(false)) => (raw_input + cached, raw_input),
            (Some(_), None) => {
                known = false;
                (raw_input, raw_input)
            }
        };
        let (input, fresh) = match (counts.cache_write, semantics.cache_write_in_input) {
            (None, _) | (Some(0), _) => (input, fresh),
            (Some(_), Some(true)) => (input, fresh.saturating_sub(written)),
            (Some(_), Some(false)) => (input + written, fresh),
            (Some(_), None) => {
                known = false;
                (input, fresh)
            }
        };
        let raw_output = counts.output.unwrap_or(0);
        let reasoning = counts.reasoning.unwrap_or(0);
        let output = match (counts.reasoning, semantics.reasoning_in_output) {
            (None, _) | (Some(0), _) | (Some(_), Some(true)) => raw_output,
            (Some(_), Some(false)) => raw_output + reasoning,
            (Some(_), None) => {
                known = false;
                raw_output
            }
        };
        totals.input += input;
        totals.cached += cached;
        totals.cache_write += written;
        totals.output += output;
        totals.reasoning += reasoning;
        if !known {
            totals.uncertain = true;
            cost = None;
            continue;
        }
        let model = measurement.report.model.as_deref().unwrap_or("");
        let price = (route == Route::Api)
            .then(|| table.lookup(provider_key(record.agent), model))
            .flatten();
        cost = match (cost, price) {
            (Some(sum), Some((price, _))) => {
                let tier = price.above_tokens.is_some_and(|above| input > above);
                let input_rate = if tier {
                    price.tier_input.unwrap_or(price.input)
                } else {
                    price.input
                };
                let output_rate = if tier {
                    price.tier_output.unwrap_or(price.output)
                } else {
                    price.output
                };
                Some(
                    sum + (fresh as f64 * input_rate
                        + cached as f64 * price.cached_input.unwrap_or(input_rate)
                        + written as f64 * price.cache_write.unwrap_or(input_rate)
                        + output as f64 * output_rate)
                        / 1_000_000.0,
                )
            }
            _ => None,
        };
    }
    if !reported {
        totals.unreported = 1;
    }
    match route {
        Route::Plan => totals.plan = 1,
        Route::Agent => totals.agent = 1,
        Route::Local => totals.local = 1,
        Route::Api if !reported => {}
        Route::Api => match cost {
            Some(usd) => {
                totals.usd = usd;
                totals.priced = 1;
            }
            None => totals.unpriced = 1,
        },
    }
    totals
}

/// Compact token count: 950, 12.4k, 3.1M.
pub fn tokens(value: u64) -> String {
    match value {
        0..=999 => value.to_string(),
        1_000..=999_999 => format!("{:.1}k", value as f64 / 1_000.0),
        _ => format!("{:.1}M", value as f64 / 1_000_000.0),
    }
}

/// `est. $0.31`, `est. $0.31+?`, `plan`, `local`, `no price`.
pub fn cost(totals: &Totals) -> String {
    if totals.priced == 0 {
        return if totals.unpriced > 0 {
            "no price"
        } else if totals.plan > 0 {
            "plan"
        } else if totals.agent > 0 {
            "agent billing"
        } else if totals.local > 0 {
            "local"
        } else if totals.unreported > 0 {
            "unreported"
        } else {
            "no requests"
        }
        .into();
    }
    let amount = if totals.usd > 0.0 && totals.usd < 0.01 {
        format!("{:.4}", totals.usd)
    } else {
        format!("{:.2}", totals.usd)
    };
    // Local requests cost nothing; every other unpriced part leaves a remainder.
    let partial = totals.unpriced + totals.plan + totals.agent > 0 || totals.lower_bound();
    format!("est. ${amount}{}", if partial { "+?" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::usage::{
        Counters, Identity, Kind, Measurement, Report, Scope, Semantics,
    };

    fn record(agent: Agent, model: &str, counters: Counters, semantics: Semantics) -> Record {
        let mut record = Record::new(
            "request-1".into(),
            1,
            crate::store::Attribution {
                partner: 1,
                name: "Eevee".into(),
                form: 0,
            },
            agent,
        );
        record.measurements.push(Measurement {
            report: Report {
                identity: Identity::default(),
                scope: Scope::Response,
                kind: Kind::Snapshot,
                counters,
                baseline: None,
                model: Some(model.into()),
                tokenizer: None,
                semantics,
                complete: true,
            },
            attributable: counters,
            delta_events: Vec::new(),
        });
        record
    }

    fn counts(
        input: u64,
        cache_read: Option<u64>,
        output: u64,
        reasoning: Option<u64>,
    ) -> Counters {
        Counters {
            input: Some(input),
            output: Some(output),
            cache_read,
            cache_write: None,
            reasoning,
        }
    }

    #[test]
    fn cached_tokens_inside_input_are_not_added_twice_and_are_priced_lower() {
        let table = Table::default();
        let semantics = Semantics {
            cache_read_in_input: Some(true),
            reasoning_in_output: Some(true),
            ..Semantics::default()
        };
        let totals = request(
            &record(
                Agent::OpenAI,
                "gpt-4.1-2025-04-14",
                counts(1_000_000, Some(400_000), 100_000, Some(50_000)),
                semantics,
            ),
            Route::Api,
            &table,
        );
        assert_eq!(
            (totals.input, totals.cached, totals.output),
            (1_000_000, 400_000, 100_000)
        );
        // 600k fresh × $2 + 400k cached × $0.5 + 100k output × $8
        assert!(
            (totals.usd - (1.2 + 0.2 + 0.8)).abs() < 1e-9,
            "{}",
            totals.usd
        );
        assert!(!totals.lower_bound());
        assert_eq!(cost(&totals), "est. $2.20");
    }

    #[test]
    fn separate_cache_writes_and_reasoning_are_added_once() {
        let table = Table::default();
        let semantics = Semantics {
            cache_read_in_input: Some(false),
            cache_write_in_input: Some(false),
            reasoning_in_output: Some(false),
        };
        let mut counters = counts(1_000, Some(2_000), 500, Some(300));
        counters.cache_write = Some(4_000);
        let totals = request(
            &record(
                Agent::Anthropic,
                "claude-sonnet-4-20250514",
                counters,
                semantics,
            ),
            Route::Api,
            &table,
        );
        assert_eq!((totals.input, totals.output), (7_000, 800));
        let expected = (1_000.0 * 3.0 + 2_000.0 * 0.3 + 4_000.0 * 3.75 + 800.0 * 15.0) / 1e6;
        assert!((totals.usd - expected).abs() < 1e-12);
    }

    #[test]
    fn unknown_subset_relations_keep_tokens_but_make_cost_unknown() {
        let table = Table::default();
        for reasoning_in_output in [Some(true), Some(false), None] {
            let semantics = Semantics {
                reasoning_in_output,
                ..Semantics::default()
            };
            let totals = request(
                &record(
                    Agent::OpenAI,
                    "gpt-5",
                    counts(10, None, 100, Some(40)),
                    semantics,
                ),
                Route::Api,
                &table,
            );
            match reasoning_in_output {
                Some(true) => assert_eq!((totals.output, totals.uncertain), (100, false)),
                Some(false) => assert_eq!((totals.output, totals.uncertain), (140, false)),
                None => {
                    assert_eq!(
                        (totals.output, totals.uncertain, totals.unpriced),
                        (100, true, 1)
                    );
                    assert_eq!(cost(&totals), "no price");
                }
            }
        }
    }

    #[test]
    fn missing_usage_and_unbaselined_cumulative_reports_are_never_zero() {
        let table = Table::default();
        let empty = Record::new(
            "request-2".into(),
            1,
            crate::store::Attribution {
                partner: 1,
                name: "Eevee".into(),
                form: 0,
            },
            Agent::OpenCode,
        );
        let totals = request(&empty, Route::Agent, &table);
        assert_eq!((totals.unreported, totals.agent), (1, 1));
        assert!(totals.lower_bound());

        let mut cumulative = record(
            Agent::OpenAI,
            "gpt-5",
            counts(10, None, 10, None),
            Semantics::default(),
        );
        cumulative.measurements[0].report.scope = Scope::Session;
        cumulative.measurements[0].attributable = Counters::default();
        let totals = request(&cumulative, Route::Api, &table);
        assert_eq!(
            (totals.tokens(), totals.unreported, totals.uncertain),
            (0, 1, true)
        );
    }

    #[test]
    fn route_kinds_and_unknown_models_are_labelled_not_priced() {
        let table = Table::default();
        let plain = Semantics::default();
        let usage = counts(1_000, None, 1_000, None);
        assert_eq!(
            cost(&request(
                &record(Agent::ChatGPT, "gpt-5", usage, plain),
                Route::Plan,
                &table
            )),
            "plan"
        );
        assert_eq!(
            cost(&request(
                &record(Agent::Compatible, "llama3", usage, plain),
                Route::Local,
                &table
            )),
            "local"
        );
        assert_eq!(
            cost(&request(
                &record(Agent::OpenAI, "gpt-9-unknown", usage, plain),
                Route::Api,
                &table
            )),
            "no price"
        );
        assert_eq!(route(Agent::Compatible, true), Route::Local);
        assert_eq!(route(Agent::Codex, false), Route::Agent);
        let mut mixed = request(
            &record(Agent::OpenAI, "gpt-5", usage, plain),
            Route::Api,
            &table,
        );
        mixed.add(&request(
            &record(Agent::ChatGPT, "gpt-5", usage, plain),
            Route::Plan,
            &table,
        ));
        assert!(cost(&mixed).ends_with("+?"));
    }

    #[test]
    fn user_rows_override_built_ins_by_longest_prefix_and_tiers_apply() {
        let table = Table {
            user: Table::parse(
                "meter = false\nbudget_usd = 1.5\n[[price]]\nprovider = \"openai\"\nmodel = \"gpt-5-mini\"\ninput = 9\noutput = 9\n",
            )
            .unwrap()
            .price,
            ..Table::default()
        };
        assert_eq!(
            table.lookup("openai", "gpt-5-mini-2025").unwrap().1,
            Source::User
        );
        assert_eq!(table.lookup("openai", "gpt-5-2025").unwrap().0.input, 1.25);
        assert_eq!(
            table.lookup("openai", "gpt-4o-mini").unwrap().0.model,
            "gpt-4o-mini"
        );
        let usage = counts(300_000, None, 1_000_000, None);
        let totals = request(
            &record(Agent::Google, "gemini-2.5-pro", usage, Semantics::default()),
            Route::Api,
            &table,
        );
        assert!((totals.usd - (0.3 * 2.5 + 15.0)).abs() < 1e-9);
        assert!(
            Table::parse("[[price]]\nprovider = \"x\"\nmodel = \"m\"\ninput = -1\noutput = 1\n")
                .is_err()
        );
        assert!(Table::parse("unknown = 1\n").is_err());
    }

    #[test]
    fn preferences_round_trip_through_the_file_and_staleness_uses_the_date() {
        let dir = std::env::temp_dir().join(format!("ev-pricing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut table = Table::load(&dir);
        assert!(table.meter && table.error.is_none());
        table.meter = false;
        table.budget_usd = Some(2.5);
        table.user = Table::parse("[[price]]\nprovider = \"compatible\"\nmodel = \"m\"\ninput = 1\noutput = 2\ncached_input = 0.5\n").unwrap().price;
        table.save_preferences().unwrap();
        let loaded = Table::load(&dir);
        assert_eq!(
            (loaded.meter, loaded.budget_usd, loaded.user_rows()),
            (false, Some(2.5), 1)
        );
        assert_eq!(
            loaded.lookup("compatible", "m").unwrap().0.cached_input,
            Some(0.5)
        );
        std::fs::write(dir.join(FILE_NAME), "meter = maybe").unwrap();
        assert!(Table::load(&dir).error.is_some());
        let _ = std::fs::remove_dir_all(&dir);
        let checked = days(BUILT_IN_AS_OF).unwrap() as u64 * 86_400;
        assert!(!built_in_stale(checked + 89 * 86_400));
        assert!(built_in_stale(checked + 91 * 86_400));
        assert_eq!(days("1970-01-01"), Some(0));
        assert_eq!(tokens(48_230), "48.2k");
    }
}
