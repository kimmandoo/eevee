pub struct Form {
    pub name: &'static str,
    pub element: &'static str,
    pub role: &'static str,
    pub evidence: &'static str,
    pub rgb: (u8, u8, u8),
    pub indexed: u8,
}

pub const FORMS: [Form; 9] = [
    Form {
        name: "Eevee",
        element: "Normal",
        role: "Intake, scope & routing",
        evidence: "Expected: bounded task and selected specialist. Architecture belongs to Espeon.",
        rgb: (220, 184, 137),
        indexed: 180,
    },
    Form {
        name: "Vaporeon",
        element: "Water",
        role: "Discovery & context assembly",
        evidence: "Expected: source-linked findings, constraints, and a context manifest. No diagnosis inferred.",
        rgb: (103, 207, 214),
        indexed: 80,
    },
    Form {
        name: "Jolteon",
        element: "Electric",
        role: "Execution & runtime verification",
        evidence: "Expected: command, cwd, code identity, output, and exit result. Execution is not a correctness verdict.",
        rgb: (231, 202, 103),
        indexed: 221,
    },
    Form {
        name: "Flareon",
        element: "Fire",
        role: "Code implementation & fixes",
        evidence: "Expected: requirement-linked patch. Request command verification from Jolteon; do not invent a passed check.",
        rgb: (234, 151, 129),
        indexed: 209,
    },
    Form {
        name: "Espeon",
        element: "Psychic",
        role: "Architecture, plans & dependencies",
        evidence: "Expected: chosen approach, tradeoffs, acceptance criteria, and dependencies. Intake belongs to Eevee.",
        rgb: (188, 166, 229),
        indexed: 183,
    },
    Form {
        name: "Umbreon",
        element: "Dark",
        role: "Failure diagnosis & security",
        evidence: "Expected: supported root cause or explicit unknown, with actionable risks. Jolteon executes diagnostics.",
        rgb: (224, 174, 91),
        indexed: 215,
    },
    Form {
        name: "Leafeon",
        element: "Grass",
        role: "Maintainability & measured efficiency",
        evidence: "Expected: bounded, behavior-preserving patch and observed validation. Behavior changes belong to Flareon.",
        rgb: (155, 192, 143),
        indexed: 108,
    },
    Form {
        name: "Glaceon",
        element: "Ice",
        role: "Code review & release readiness",
        evidence: "Expected: evidence-linked findings. Explicit independent Review stays read-only; ordinary file changes require exact consent.",
        rgb: (144, 204, 233),
        indexed: 117,
    },
    Form {
        name: "Sylveon",
        element: "Fairy",
        role: "Handoff, docs & curated memory",
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
