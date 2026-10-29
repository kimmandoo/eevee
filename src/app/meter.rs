//! Live token and estimated-cost meter (XC09). Launch totals live in memory and
//! reset on restart; task totals add this task's saved usage records. Every
//! amount is a labelled local estimate from reported counters, never billing.
use super::*;
use crate::workspace::{
    pricing::{self, Route, Totals},
    usage::Record,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct State {
    pub table: pricing::Table,
    /// Every request dispatched in this launch, with the route it used.
    launch: BTreeMap<String, (Record, Route)>,
    routes: Vec<(crate::providers::Agent, Route)>,
    budget_noticed: bool,
}

#[derive(Default)]
pub struct Summary {
    pub request: Option<Totals>,
    pub task: Totals,
    pub launch: Totals,
    pub partners: Vec<(String, Totals)>,
    pub models: Vec<(String, Totals)>,
}

impl State {
    fn route(&mut self, agent: crate::providers::Agent) -> Route {
        if let Some((_, route)) = self.routes.iter().find(|(known, _)| *known == agent) {
            return *route;
        }
        let loopback = agent == crate::providers::Agent::Compatible
            && crate::providers::direct::config::loopback(agent);
        let route = pricing::route(agent, loopback);
        self.routes.push((agent, route));
        route
    }
    fn cached_route(&self, agent: crate::providers::Agent) -> Route {
        self.routes
            .iter()
            .find(|(known, _)| *known == agent)
            .map_or_else(|| pricing::route(agent, false), |(_, route)| *route)
    }
}

fn model(record: &Record) -> String {
    record
        .measurements
        .iter()
        .rev()
        .find_map(|measurement| measurement.report.model.clone())
        .unwrap_or_else(|| "unreported model".into())
}

impl App {
    pub fn load_meter(&mut self, data_dir: &std::path::Path) {
        self.meter.table = pricing::Table::load(data_dir);
    }

    /// Copies every partner's current request record into the launch totals.
    /// Records only grow from provider reports; nothing is estimated here.
    pub(super) fn observe_usage(&mut self) -> bool {
        let records: Vec<Record> = std::iter::once(&self.planning)
            .chain(self.cooperation.parked.values())
            .filter_map(|state| state.usage.record.clone())
            .collect();
        let mut changed = false;
        for record in records {
            if self
                .meter
                .launch
                .get(&record.request_id)
                .is_some_and(|(known, _)| *known == record)
            {
                continue;
            }
            let route = self.meter.route(record.agent);
            self.meter
                .launch
                .insert(record.request_id.clone(), (record, route));
            changed = true;
        }
        if changed
            && !self.meter.budget_noticed
            && let Some(budget) = self.meter.table.budget_usd
        {
            let spent = self.meter_summary().launch.usd;
            if spent >= budget {
                self.meter.budget_noticed = true;
                self.notice = format!(
                    "Usage budget: this launch's estimate est. ${spent:.2} reached ${budget:.2}. Work continues; /usage for details."
                );
            }
        }
        changed
    }

    fn meter_totals(&self, record: &Record, route: Route) -> Totals {
        pricing::request(record, route, &self.meter.table)
    }

    pub fn meter_summary(&self) -> Summary {
        let mut summary = Summary::default();
        let mut partners: BTreeMap<String, Totals> = BTreeMap::new();
        let mut models: BTreeMap<String, Totals> = BTreeMap::new();
        for (record, route) in self.meter.launch.values() {
            let totals = self.meter_totals(record, *route);
            summary.launch.add(&totals);
            if record.task_id == self.task_id {
                summary.task.add(&totals);
            }
            partners
                .entry(format!("#{} {}", record.owner.partner, record.owner.name))
                .or_default()
                .add(&totals);
            models.entry(model(record)).or_default().add(&totals);
        }
        for record in &self.records.snapshot.usages {
            if record.task_id == self.task_id && !self.meter.launch.contains_key(&record.request_id)
            {
                let route = self.meter.cached_route(record.agent);
                summary.task.add(&self.meter_totals(record, route));
            }
        }
        summary.request = self.planning.usage.record.as_ref().map(|record| {
            let route = self.meter.cached_route(record.agent);
            self.meter_totals(record, route)
        });
        summary.partners = partners.into_iter().collect();
        summary.models = models.into_iter().collect();
        summary
    }

    /// The status-row chip, or None when hidden, empty or too narrow.
    pub fn meter_chip(&self, width: u16) -> Option<String> {
        if !self.meter.table.meter || width < 70 || self.meter.launch.is_empty() {
            return None;
        }
        let summary = self.meter_summary();
        let launch = &summary.launch;
        let bound = if launch.lower_bound() { "≥" } else { "" };
        let tokens = pricing::tokens(launch.tokens());
        let cost = pricing::cost(launch);
        if width < 90 {
            let cost = cost.strip_prefix("est. ").unwrap_or(&cost);
            return Some(format!("{bound}{tokens} · {cost}"));
        }
        let request = summary
            .request
            .filter(|_| self.planning.busy())
            .map(|request| format!("Req {} ↑ · ", pricing::tokens(request.tokens())))
            .unwrap_or_default();
        Some(format!("{request}Launch {bound}{tokens} tok · {cost}"))
    }

    pub(super) fn usage_command(&mut self, name: &str, argument: Option<&str>) -> bool {
        match name {
            "usage" => {
                self.open_overlay(Overlay::Usage);
            }
            "usage-meter" => {
                let enabled = match argument {
                    Some("on") => true,
                    Some("off") => false,
                    _ => !self.meter.table.meter,
                };
                self.meter.table.meter = enabled;
                self.save_meter(format!(
                    "Usage meter {}. /usage shows totals either way.",
                    if enabled { "shown" } else { "hidden" }
                ));
            }
            "usage-budget" => {
                let budget = match argument.map(|value| value.trim_start_matches('$')) {
                    None | Some("off") => None,
                    Some(value) => match value.parse::<f64>() {
                        Ok(budget) if budget.is_finite() && (0.0..=10_000.0).contains(&budget) => {
                            Some(budget)
                        }
                        _ => {
                            self.notice =
                                "Usage budget needs a USD amount (e.g. /usage-budget 2.50) or off."
                                    .into();
                            return true;
                        }
                    },
                };
                self.meter.table.budget_usd = budget;
                self.meter.budget_noticed = false;
                self.save_meter(match budget {
                    Some(budget) => format!(
                        "Usage budget ${budget:.2}: one notice when this launch's estimate reaches it; never blocks work."
                    ),
                    None => "Usage budget off.".into(),
                });
            }
            "prices" => {
                let data_dir = self
                    .meter
                    .table
                    .path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default();
                self.meter.table = pricing::Table::load(&data_dir);
                self.notice = match &self.meter.table.error {
                    Some(error) => format!("Prices not loaded: {error}"),
                    None => format!(
                        "Prices reloaded: {} user row(s) from {}; built-in list prices as of {}.",
                        self.meter.table.user_rows(),
                        self.meter.table.path.display(),
                        pricing::BUILT_IN_AS_OF
                    ),
                };
                self.open_overlay(Overlay::Usage);
            }
            _ => return false,
        }
        true
    }

    fn save_meter(&mut self, success: String) {
        self.notice = match self.meter.table.save_preferences() {
            Ok(()) => success,
            Err(error) => format!("{success} Not saved: {error}"),
        };
    }

    pub(super) fn usage_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return true;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_overlay(),
            KeyCode::Char('r') => {
                self.usage_command("prices", None);
            }
            KeyCode::Char('m') => {
                self.usage_command("usage-meter", None);
            }
            _ => return false,
        }
        true
    }

    /// One owner's reported usage for the party board; never a fixed "unknown".
    pub(super) fn owner_usage_line(&self, partner: i64) -> String {
        let Some(record) = self
            .planning_state(partner)
            .and_then(|state| state.usage.record.as_ref())
        else {
            return "Provider usage/cost: no request dispatched yet.".into();
        };
        let totals = self.meter_totals(record, self.meter.cached_route(record.agent));
        if totals.unreported > 0 {
            return "Provider usage/cost: unknown (provider has not reported usage).".into();
        }
        format!(
            "Provider usage: {} in · {} cached · {} out{} · {} (local estimate, not billing).",
            totals.input,
            totals.cached,
            totals.output,
            if totals.uncertain {
                " (partly unattributed)"
            } else {
                ""
            },
            pricing::cost(&totals)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::usage::{
        Counters, Identity, Kind, Measurement, Report, Scope, Semantics,
    };

    fn record(id: &str, task_id: i64, partner: i64, input: u64, output: u64) -> Record {
        let counters = Counters {
            input: Some(input),
            output: Some(output),
            ..Counters::default()
        };
        let mut record = Record::new(
            id.into(),
            task_id,
            store::Attribution {
                partner,
                name: format!("P{partner}"),
                form: 0,
            },
            crate::providers::Agent::OpenAI,
        );
        record.measurements.push(Measurement {
            report: Report {
                identity: Identity::default(),
                scope: Scope::Response,
                kind: Kind::Snapshot,
                counters,
                baseline: None,
                model: Some("gpt-4.1".into()),
                tokenizer: None,
                semantics: Semantics::default(),
                complete: true,
            },
            attributable: counters,
            delta_events: Vec::new(),
        });
        record
    }

    #[test]
    fn launch_spans_tasks_and_partners_while_task_adds_saved_records() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.task_id = 1;
        app.planning.usage.record = Some(record("r1", 1, 1, 1_000_000, 0));
        assert!(app.observe_usage());
        assert!(!app.observe_usage(), "unchanged records are not re-added");
        app.planning.usage.record = Some(record("r2", 2, 2, 0, 1_000_000));
        assert!(app.observe_usage());
        app.records.snapshot.usages =
            vec![record("old", 1, 1, 500_000, 0), record("r1", 1, 1, 1, 1)];
        let summary = app.meter_summary();
        assert_eq!(summary.launch.tokens(), 2_000_000);
        assert!((summary.launch.usd - (2.0 + 8.0)).abs() < 1e-9);
        // Saved "old" adds to the task; the saved copy of r1 does not double count.
        assert_eq!(summary.task.input, 1_500_000);
        assert_eq!(summary.partners.len(), 2);
        assert_eq!(
            app.meter_chip(120).unwrap(),
            "Launch 2.0M tok · est. $10.00"
        );
        assert_eq!(app.meter_chip(80).unwrap(), "2.0M · $10.00");
        assert_eq!(app.meter_chip(60), None);
        app.meter.table.meter = false;
        assert_eq!(app.meter_chip(120), None);
    }

    #[test]
    fn unreported_requests_make_the_chip_a_lower_bound_and_budget_notices_once() {
        let mut app = App::new("/workspace".into(), 120, 36);
        app.meter.table.budget_usd = Some(1.0);
        app.planning.usage.record = Some(record("r1", 1, 1, 1_000_000, 0));
        app.observe_usage();
        assert!(app.notice.contains("Usage budget"), "{}", app.notice);
        app.notice.clear();
        app.planning.usage.record = Some(record("r2", 1, 1, 1_000_000, 0));
        app.observe_usage();
        assert!(app.notice.is_empty(), "budget notice repeats");
        let mut empty = record("r3", 1, 1, 0, 0);
        empty.measurements.clear();
        app.planning.usage.record = Some(empty);
        app.observe_usage();
        assert_eq!(
            app.meter_chip(120).unwrap(),
            "Launch ≥2.0M tok · est. $4.00+?"
        );
    }
}
