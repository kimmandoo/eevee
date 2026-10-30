pub struct Form {
    pub name: &'static str,
    pub element: &'static str,
    pub role: &'static str,
    /// One plain sentence: what this partner does for you.
    pub summary: &'static str,
    /// Example requests that bring this form out.
    pub good_for: &'static str,
    pub evidence: &'static str,
    pub rgb: (u8, u8, u8),
    pub indexed: u8,
}

pub const FORMS: [Form; 9] = [
    Form {
        name: "Eevee",
        element: "Normal",
        role: "Intake, scope & routing",
        summary: "All-rounder. Takes a new request, works out what it needs and picks the right specialist.",
        good_for: "starting a task, general questions",
        evidence: "Expected: bounded task and selected specialist. Architecture belongs to Espeon.",
        rgb: (220, 184, 137),
        indexed: 180,
    },
    Form {
        name: "Vaporeon",
        element: "Water",
        role: "Discovery & context assembly",
        summary: "Explorer. Reads the code and finds where things are, without changing anything.",
        good_for: "\"where is…\", \"how does … work\", \"explain this code\"",
        evidence: "Expected: source-linked findings, constraints, and a context manifest. No diagnosis inferred.",
        rgb: (103, 207, 214),
        indexed: 80,
    },
    Form {
        name: "Jolteon",
        element: "Electric",
        role: "Execution & runtime verification",
        summary: "Runner. Builds, tests and runs commands, and shows you the real output.",
        good_for: "\"run the tests\", \"build it\", \"run the linter\"",
        evidence: "Expected: command, cwd, code identity, output, and exit result. Execution is not a correctness verdict.",
        rgb: (231, 202, 103),
        indexed: 221,
    },
    Form {
        name: "Flareon",
        element: "Fire",
        role: "Code implementation & fixes",
        summary: "Builder. Writes and fixes code: new features, bug fixes and edits.",
        good_for: "\"add …\", \"fix …\", \"change …\"",
        evidence: "Expected: requirement-linked patch. Request command verification from Jolteon; do not invent a passed check.",
        rgb: (234, 151, 129),
        indexed: 209,
    },
    Form {
        name: "Espeon",
        element: "Psychic",
        role: "Architecture, plans & dependencies",
        summary: "Planner. Designs the approach and weighs the options before code is written.",
        good_for: "\"how should we …\", \"plan …\", \"compare …\"",
        evidence: "Expected: chosen approach, tradeoffs, acceptance criteria, and dependencies. Intake belongs to Eevee.",
        rgb: (188, 166, 229),
        indexed: 183,
    },
    Form {
        name: "Umbreon",
        element: "Dark",
        role: "Failure diagnosis & security",
        summary: "Detective. Finds out why something breaks: errors, crashes, slowness, security risks.",
        good_for: "\"why does it crash\", \"the build is broken\"",
        evidence: "Expected: supported root cause or explicit unknown, with actionable risks. Jolteon executes diagnostics.",
        rgb: (224, 174, 91),
        indexed: 215,
    },
    Form {
        name: "Leafeon",
        element: "Grass",
        role: "Maintainability & measured efficiency",
        summary: "Gardener. Cleans up and speeds up code without changing what it does.",
        good_for: "\"refactor …\", \"clean up …\", \"make … faster\"",
        evidence: "Expected: bounded, behavior-preserving patch and observed validation. Behavior changes belong to Flareon.",
        rgb: (155, 192, 143),
        indexed: 108,
    },
    Form {
        name: "Glaceon",
        element: "Ice",
        role: "Code review & release readiness",
        summary: "Reviewer. Checks changes for bugs and risks before you merge or release.",
        good_for: "\"review my changes\", \"is this safe to merge\"",
        evidence: "Expected: evidence-linked findings. Explicit independent Review stays read-only; ordinary file changes require exact consent.",
        rgb: (144, 204, 233),
        indexed: 117,
    },
    Form {
        name: "Sylveon",
        element: "Fairy",
        role: "Handoff, docs & curated memory",
        summary: "Writer. Writes docs, summaries and handoff notes so work can be picked up later.",
        good_for: "\"update the README\", \"summarize what we did\"",
        evidence: "Expected: evidence-linked handoff and explicitly accepted memories. Never infer user preferences or success.",
        rgb: (221, 165, 197),
        indexed: 218,
    },
];

/// Empty-surface guidance per form (EV04): the signature `object → action`
/// from DESIGN_RULES, then one primary action that opens an existing flow.
/// `None` means the composer itself is the flow. Opening never executes.
pub const FIRST_ACTIONS: [(&str, Option<&str>, &str); 9] = [
    (
        "Task card → edit criteria, choose next form",
        Some("criteria-edit"),
        "Edit criteria",
    ),
    (
        "File/range → inspect, pin, exclude",
        Some("files"),
        "Attach a file",
    ),
    (
        "Command request → inspect, approve/deny, cancel",
        Some("command-edit"),
        "Author a command",
    ),
    (
        "Requirement + approved paths → request a change",
        Some("paths"),
        "Choose write paths",
    ),
    (
        "Design question → compare alternatives, choose approach",
        None,
        "Type the design question below",
    ),
    (
        "Failure → add hypothesis, request diagnostic run",
        Some("workflow-diagnosis"),
        "Add a hypothesis",
    ),
    (
        "Bounded hotspot → compare baseline, refactor, measure",
        Some("workflow-refactor"),
        "Choose a hotspot",
    ),
    (
        "Change set → inspect criteria, findings, verification age",
        Some("workflow-review"),
        "Open the review desk",
    ),
    (
        "Verified task facts → preview handoff, propose a memory",
        Some("handoff"),
        "Preview the handoff",
    ),
];

/// Requestable capabilities, not grants. Every form supports project discovery
/// and guarded coding; operation policy and exact approval remain authoritative.
pub const CAPABILITIES: [&str; 9] = {
    let mut capabilities = ["read · edit"; 9];
    capabilities[2] = "read · edit · run";
    capabilities
};
