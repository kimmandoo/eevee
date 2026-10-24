//! Rust expression grammar v1, intentionally syntactic (not binding/type analysis).
//! Patterns accept calls, method calls (no turbofish), paths, literals, parens,
//! unary/binary/reference, field/index, tuple/array, await and try expressions.
//! `$NAME` is a whole expression-node capture; names are ASCII [A-Z][A-Z0-9_]*.
//! Repetition compares token trees, ignoring comments/whitespace but preserving
//! literal spelling and delimiters. No variadic captures, statements, patterns,
//! macro-body traversal, expansion, rewriting, or implicit context selection.
use super::observations::Frozen;
use super::*;
use crate::workspace::context::symbols::{
    PARSE_EXPRESSION_TOKENS as EXPRESSION_TOKENS, PARSE_NESTING as NESTING,
    PARSE_NON_WHITESPACE_BYTES as TOKENS, PARSE_OPENINGS as LEXICAL_OPENINGS,
    check_parse_bound as lexical_bound,
};
use proc_macro2::{Span, TokenStream, TokenTree};
use syn::{Expr, spanned::Spanned, visit::Visit};

const PREFIX: &str = "__eevee_structural_capture_";
const MATCHES: usize = 256;
const PAGE: usize = 32;
const RESPONSE_BYTES: usize = 64 * 1024;

pub(super) struct Pattern {
    expression: Expr,
    text: String,
}
impl Pattern {
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() || text.len() > 2048 || !safe_text(text) || text.contains(PREFIX) {
            return Err(
                "Pattern requires 1–2048 safe UTF-8 bytes and no reserved capture prefix".into(),
            );
        }
        lexical_bound(text)?;
        let tokens: TokenStream = text
            .parse()
            .map_err(|_| "Pattern has invalid Rust tokens")?;
        let mut names = BTreeSet::new();
        let replaced = replace_captures(tokens, &mut names)?.to_string();
        if names.len() > 16 {
            return Err("Pattern permits at most 16 named captures".into());
        }
        let expression =
            syn::parse_str::<Expr>(&replaced).map_err(|_| "Pattern is not one Rust expression")?;
        let mut grammar = Grammar {
            valid: true,
            captures: BTreeSet::new(),
        };
        grammar.visit_expr(&expression);
        if !grammar.valid || grammar.captures != names {
            return Err("Unsupported expression grammar or capture position; use whole expression-node captures and the advertised Rust grammar".into());
        }
        Ok(Self {
            expression,
            text: replaced,
        })
    }
}

fn replace_captures(
    tokens: TokenStream,
    names: &mut BTreeSet<String>,
) -> Result<TokenStream, String> {
    let mut output = TokenStream::new();
    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Punct(punctuation) if punctuation.as_char() == '$' => {
                let Some(TokenTree::Ident(name)) = tokens.next() else {
                    return Err("$ must precede an ASCII capture name".into());
                };
                let name = name.to_string();
                if !name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
                {
                    return Err("Capture names use [A-Z][A-Z0-9_]*".into());
                }
                names.insert(name.clone());
                output.extend([TokenTree::Ident(proc_macro2::Ident::new(
                    &format!("{PREFIX}{name}"),
                    punctuation.span(),
                ))]);
            }
            TokenTree::Group(group) => {
                let mut replaced = proc_macro2::Group::new(
                    group.delimiter(),
                    replace_captures(group.stream(), names)?,
                );
                replaced.set_span(group.span());
                output.extend([TokenTree::Group(replaced)]);
            }
            token => output.extend([token]),
        }
    }
    Ok(output)
}
fn capture(expression: &Expr) -> Option<String> {
    if let Expr::Path(path) = expression
        && path.qself.is_none()
        && path.path.leading_colon.is_none()
        && path.path.segments.len() == 1
        && matches!(path.path.segments[0].arguments, syn::PathArguments::None)
    {
        return path.path.segments[0]
            .ident
            .to_string()
            .strip_prefix(PREFIX)
            .map(str::to_owned);
    }
    None
}
struct Grammar {
    valid: bool,
    captures: BTreeSet<String>,
}
impl<'ast> Visit<'ast> for Grammar {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let Some(name) = capture(expression) {
            self.captures.insert(name);
            return;
        }
        self.valid &= matches!(
            expression,
            Expr::Call(_)
                | Expr::MethodCall(_)
                | Expr::Path(_)
                | Expr::Lit(_)
                | Expr::Paren(_)
                | Expr::Unary(_)
                | Expr::Binary(_)
                | Expr::Reference(_)
                | Expr::Field(_)
                | Expr::Index(_)
                | Expr::Tuple(_)
                | Expr::Array(_)
                | Expr::Await(_)
                | Expr::Try(_)
        );
        if let Expr::MethodCall(method) = expression {
            self.valid &= method.turbofish.is_none();
        }
        if self.valid {
            syn::visit::visit_expr(self, expression);
        }
    }
}

fn range(text: &str, span: Span) -> Option<std::ops::Range<usize>> {
    let bytes = span.byte_range();
    text.get(bytes.clone()).map(|_| bytes)
}
fn token_identity(text: &str, span: Span) -> Option<String> {
    let value = text.get(range(text, span)?)?;
    let tokens = value.parse::<TokenStream>().ok()?;
    // TokenStream rendering preserves literal spelling and delimiter/group
    // structure while discarding whitespace/comments. Joint punctuation is kept.
    Some(tokens.to_string())
}
fn same(left: Span, right: Span, pattern: &str, source: &str) -> bool {
    match (token_identity(pattern, left), token_identity(source, right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}
#[derive(Clone)]
struct Capture {
    range: std::ops::Range<usize>,
    identity: String,
    span: Span,
}
fn matches_expression(
    pattern: &Expr,
    source: &Expr,
    pattern_text: &str,
    source_text: &str,
    captures: &mut BTreeMap<String, Capture>,
    control: &ObservationControl<'_>,
) -> bool {
    if control.check().is_err() {
        return false;
    }
    if let Some(name) = capture(pattern) {
        let Some(identity) = token_identity(source_text, source.span()) else {
            return false;
        };
        if let Some(previous) = captures.get(&name) {
            return previous.identity == identity;
        }
        let Some(range) = range(source_text, source.span()) else {
            return false;
        };
        captures.insert(
            name,
            Capture {
                range,
                identity,
                span: source.span(),
            },
        );
        return true;
    }
    let equal = |left, right| same(left, right, pattern_text, source_text);
    let recur = |left: &Expr, right: &Expr, captures: &mut BTreeMap<String, Capture>| {
        matches_expression(left, right, pattern_text, source_text, captures, control)
    };
    match (pattern, source) {
        (Expr::Call(left), Expr::Call(right)) => {
            left.args.len() == right.args.len()
                && recur(&left.func, &right.func, captures)
                && left
                    .args
                    .iter()
                    .zip(&right.args)
                    .all(|(left, right)| recur(left, right, captures))
        }
        (Expr::MethodCall(left), Expr::MethodCall(right)) => {
            right.turbofish.is_none()
                && left.method == right.method
                && left.args.len() == right.args.len()
                && recur(&left.receiver, &right.receiver, captures)
                && left
                    .args
                    .iter()
                    .zip(&right.args)
                    .all(|(left, right)| recur(left, right, captures))
        }
        (Expr::Path(left), Expr::Path(right)) => equal(left.span(), right.span()),
        (Expr::Lit(left), Expr::Lit(right)) => equal(left.lit.span(), right.lit.span()),
        (Expr::Paren(left), Expr::Paren(right)) => recur(&left.expr, &right.expr, captures),
        (Expr::Unary(left), Expr::Unary(right)) => {
            equal(left.op.span(), right.op.span()) && recur(&left.expr, &right.expr, captures)
        }
        (Expr::Binary(left), Expr::Binary(right)) => {
            equal(left.op.span(), right.op.span())
                && recur(&left.left, &right.left, captures)
                && recur(&left.right, &right.right, captures)
        }
        (Expr::Reference(left), Expr::Reference(right)) => {
            left.mutability.is_some() == right.mutability.is_some()
                && recur(&left.expr, &right.expr, captures)
        }
        (Expr::Field(left), Expr::Field(right)) => {
            equal(left.member.span(), right.member.span())
                && recur(&left.base, &right.base, captures)
        }
        (Expr::Index(left), Expr::Index(right)) => {
            recur(&left.expr, &right.expr, captures) && recur(&left.index, &right.index, captures)
        }
        (Expr::Tuple(left), Expr::Tuple(right)) => {
            left.elems.len() == right.elems.len()
                && left
                    .elems
                    .iter()
                    .zip(&right.elems)
                    .all(|(left, right)| recur(left, right, captures))
        }
        (Expr::Array(left), Expr::Array(right)) => {
            left.elems.len() == right.elems.len()
                && left
                    .elems
                    .iter()
                    .zip(&right.elems)
                    .all(|(left, right)| recur(left, right, captures))
        }
        (Expr::Await(left), Expr::Await(right)) => recur(&left.base, &right.base, captures),
        (Expr::Try(left), Expr::Try(right)) => recur(&left.expr, &right.expr, captures),
        _ => false,
    }
}

/// Mask only parser-discarded file prefixes with equal-length whitespace. syn's
/// parse_file otherwise shifts every byte span after BOM/shebang removal.
fn parser_source(text: &str) -> std::borrow::Cow<'_, str> {
    let bom = if text.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let mut end = bom;
    let body = &text[bom..];
    if let Some(tail) = body.strip_prefix("#!") {
        let attribute = prefix_is_attribute(tail);
        if !attribute {
            end = bom + body.find('\n').unwrap_or(body.len());
        }
    }
    if end == 0 {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut bytes = text.as_bytes().to_vec();
    for byte in &mut bytes[..end] {
        if !matches!(*byte, b'\r' | b'\n') {
            *byte = b' ';
        }
    }
    std::borrow::Cow::Owned(String::from_utf8(bytes).expect("only complete prefixes were masked"))
}

fn prefix_is_attribute(mut text: &str) -> bool {
    loop {
        text = text.trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, '\u{200e}' | '\u{200f}')
        });
        if text.starts_with("//")
            && !text.starts_with("//!")
            && (!text.starts_with("///") || text.starts_with("////"))
        {
            let Some(end) = text.find('\n') else {
                return false;
            };
            text = &text[end + 1..];
        } else if text.starts_with("/**/") {
            text = &text[4..];
        } else if text.starts_with("/*")
            && !text.starts_with("/*!")
            && (!text.starts_with("/**") || text.starts_with("/***"))
        {
            let bytes = text.as_bytes();
            let (mut depth, mut end) = (1usize, 2usize);
            while depth > 0 && end + 1 < bytes.len() {
                match &bytes[end..end + 2] {
                    b"/*" => {
                        depth += 1;
                        end += 2;
                    }
                    b"*/" => {
                        depth -= 1;
                        end += 2;
                    }
                    _ => end += 1,
                }
            }
            if depth > 0 {
                return false;
            }
            text = &text[end..];
        } else {
            return text.starts_with('[');
        }
    }
}
fn location(text: &str, span: Span) -> Option<Value> {
    let bytes = range(text, span)?;
    let column = |position: proc_macro2::LineColumn| {
        position
            .column
            .saturating_sub(if position.line == 1 && text.starts_with('\u{feff}') {
                2
            } else {
                0
            })
            + 1
    };
    Some(json!({"start_byte":bytes.start,"end_byte":bytes.end,
        "start_line":span.start().line,"start_column":column(span.start()),
        "end_line":span.end().line,"end_column":column(span.end()),
        "range_convention":"UTF-8 byte offsets zero-based half-open; lines and Unicode-scalar columns one-based, end exclusive"}))
}
struct Searcher<'a> {
    pattern: &'a Pattern,
    source: &'a super::observations::Captured,
    matches: &'a mut Vec<Value>,
    control: &'a ObservationControl<'a>,
    limited: bool,
    range_failed: bool,
    capture_limited: bool,
}
impl<'ast> Visit<'ast> for Searcher<'_> {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if self.limited || self.control.check().is_err() {
            return;
        }
        let mut captures = BTreeMap::new();
        if matches_expression(
            &self.pattern.expression,
            expression,
            &self.pattern.text,
            &self.source.text,
            &mut captures,
            self.control,
        ) {
            if captures
                .values()
                .map(|capture| capture.range.len())
                .sum::<usize>()
                > 32 * 1024
            {
                self.capture_limited = true;
                syn::visit::visit_expr(self, expression);
                return;
            }
            if self.matches.len() >= MATCHES {
                self.limited = true;
                return;
            }
            if let Some(location) = location(&self.source.text, expression.span()) {
                let captures: BTreeMap<_,_> = captures.into_iter().map(|(name,capture)| {
                    let value = json!({"range":location_for_capture(&self.source.text, &capture),"text":&self.source.text[capture.range]});
                    (name,value)
                }).collect();
                self.matches.push(json!({"path":self.source.path,"sha256":self.source.digest,"digest_scope":"full_file",
                    "range":location,"captures":captures}));
            } else {
                self.range_failed = true;
            }
        }
        syn::visit::visit_expr(self, expression);
    }
    // syn already treats macro tokens as opaque; do not descend into token bodies.
    fn visit_macro(&mut self, _: &'ast syn::Macro) {}
}
fn location_for_capture(text: &str, capture: &Capture) -> Value {
    location(text, capture.span).expect("capture range was checked before matching")
}

pub(super) fn snapshot_identity(tool: &FileTool, source_fingerprint: &str) -> String {
    let FileTool::StructuralSearch {
        directory,
        language,
        pattern,
        ..
    } = tool
    else {
        unreachable!()
    };
    let query = json!({"operation_version":super::observations::VERSION,"directory":directory,"language":language,"pattern":pattern,"source_fingerprint":source_fingerprint});
    format!("{:x}", Sha256::digest(query.to_string().as_bytes()))
}
pub(super) fn search(
    tool: &FileTool,
    frozen: &Frozen,
    validated: Option<Pattern>,
    control: &ObservationControl<'_>,
) -> Result<Value, String> {
    control.check()?;
    let FileTool::StructuralSearch {
        directory,
        pattern,
        offset,
        snapshot,
        ..
    } = tool
    else {
        unreachable!()
    };
    let pattern = match validated {
        Some(pattern) => pattern,
        None => Pattern::parse(pattern)?,
    };
    let mut matches = Vec::new();
    let mut omissions = frozen.omissions.clone();
    let mut parsed = 0;
    let mut parse_failures = 0;
    for source in &frozen.sources {
        control.check()?;
        let parser_text = parser_source(&source.text);
        if let Err(reason) = lexical_bound(&parser_text) {
            let kind = if reason == "Invalid Rust tokens" {
                parse_failures += 1;
                "parse_failure"
            } else {
                "parser_lexical_limit"
            };
            omissions.push(
                json!({"reason":kind,"path":source.path,"sha256":source.digest,"message":reason}),
            );
            continue;
        }
        control.check()?;
        let syntax = match syn::parse_str::<syn::File>(&parser_text) {
            Ok(syntax) => syntax,
            Err(_) => {
                parse_failures += 1;
                omissions.push(
                    json!({"reason":"parse_failure","path":source.path,"sha256":source.digest}),
                );
                continue;
            }
        };
        parsed += 1;
        control.check()?;
        let mut searcher = Searcher {
            pattern: &pattern,
            source,
            matches: &mut matches,
            control,
            limited: false,
            range_failed: false,
            capture_limited: false,
        };
        searcher.visit_file(&syntax);
        control.check()?;
        if searcher.range_failed {
            omissions.push(json!({"reason":"unavailable_source_range"}));
        }
        if searcher.capture_limited {
            omissions.push(
                json!({"reason":"capture_response_byte_limit","capture_bytes_per_match":32768}),
            );
        }
        if searcher.limited {
            omissions.push(json!({"reason":"match_limit"}));
            break;
        }
    }
    control.check()?;
    if *offset > matches.len() {
        return Err("Structural offset exceeds this frozen result set".into());
    }
    if *offset > 0 && snapshot.is_empty() {
        return Err("Structural paging requires a snapshot".into());
    }
    let complete = omissions.is_empty();
    let status = if !complete {
        if parsed == 0 && parse_failures > 0 && parse_failures == frozen.sources.len() {
            "parse_failure"
        } else {
            "incomplete"
        }
    } else if matches.is_empty() {
        "no_match"
    } else {
        "success"
    };
    let message = match status {
        "success" => {
            "Actual bounded Rust expression matches within the requested project-relative directory; inspect next_offset for further retained matches; no write authority granted"
        }
        "no_match" => {
            "No matching Rust expressions in the complete privacy-filtered source scan within the requested project-relative directory; not semantic proof of absence"
        }
        "parse_failure" => {
            "All captured Rust sources failed parsing; no complete structural result or proof of absence is available"
        }
        "incomplete" => {
            "Structural observation is incomplete because source, parser, match or capture bounds omitted results; inspect omissions; no proof of absence"
        }
        _ => unreachable!(),
    };
    let source_values = frozen.sources.iter().map(|source| json!({"path":source.path,"sha256":source.digest,"bytes":source.text.len(),"digest_scope":"full_file"}));
    let (sources, omitted_sources) = bounded_metadata(source_values, 8192)?;
    let (omissions, omitted_omissions) = bounded_metadata(omissions.into_iter(), 8192)?;
    let mut value = json!({"status":status,"path":directory,"message":message,"complete":complete,"snapshot":snapshot_identity(tool,&frozen.fingerprint),
        "matches":[],"offset":offset,"next_offset":null,"total_retained_matches":matches.len(),
        "sources":sources,"omissions":omissions,"parsed_files":parsed,"scanned_source_count":frozen.sources.len(),
        "omitted_source_metadata":omitted_sources,"omitted_omission_details":omitted_omissions,
        "scanned_source_bytes":frozen.sources.iter().map(|source|source.text.len()).sum::<usize>(),
        "bounds":{"candidates":128,"source_bytes":262144,"scanned_bytes":2097152,"matches":256,"page_matches":32,"response_bytes":RESPONSE_BYTES,"pattern_bytes":2048,"captures":16,"capture_bytes_per_match":32768,"source_metadata_bytes":8192,"omission_metadata_bytes":8192,"lexical_nesting":NESTING,"lexical_delimiter_openings":LEXICAL_OPENINGS,"expression_path_tokens":EXPRESSION_TOKENS,"lexical_non_whitespace_bytes":TOKENS},
        "meaning":"Syntactic Rust expression observations, not semantic binding, rewriting, compilation or proof of absence when incomplete. Comments/strings and macro token bodies are not expression matches."});
    if omitted_sources > 0 || omitted_omissions > 0 {
        value["scan_complete"] = json!(complete);
        value["complete"] = json!(false);
        value["status"] = json!("incomplete");
        value["message"] = json!(
            "Structural observation metadata exceeded its retention bound; source or omission details were omitted; no complete result or proof of absence is available"
        );
    }
    let metadata_bytes = serde_json::to_vec(&value)
        .map_err(|_| "Cannot bound structural metadata")?
        .len();
    let mut response_bytes = metadata_bytes + 2048;
    let mut returned = Vec::new();
    for found in matches.into_iter().skip(*offset).take(PAGE) {
        let bytes = serde_json::to_vec(&found)
            .map_err(|_| "Cannot bound structural match")?
            .len()
            + 1;
        if response_bytes + bytes > RESPONSE_BYTES {
            break;
        }
        response_bytes += bytes;
        returned.push(found);
    }
    let next = offset + returned.len();
    let empty = returned.is_empty();
    value["matches"] = Value::Array(returned);
    if next < value["total_retained_matches"].as_u64().unwrap_or(0) as usize {
        if empty {
            return Err(
                "A structural match/capture exceeds the response bound; choose a narrower pattern"
                    .into(),
            );
        }
        value["next_offset"] = json!(next);
    }
    if serde_json::to_vec(&value)
        .map_err(|_| "Cannot bound structural response")?
        .len()
        > RESPONSE_BYTES - 2048
    {
        return Err("Structural source/omission metadata exceeds the response bound; choose a narrower directory".into());
    }
    Ok(value)
}

fn bounded_metadata(
    values: impl Iterator<Item = Value>,
    limit: usize,
) -> Result<(Vec<Value>, usize), String> {
    let mut bytes = 0;
    let mut retained = Vec::new();
    let mut omitted = 0;
    for value in values {
        let size = serde_json::to_vec(&value)
            .map_err(|_| "Cannot bound structural metadata")?
            .len()
            + 1;
        if bytes + size <= limit {
            bytes += size;
            retained.push(value);
        } else {
            omitted += 1;
        }
    }
    Ok((retained, omitted))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pattern_grammar_rejects_nonexpression_and_bad_capture_positions() {
        assert!(Pattern::parse("call($X, $X)").is_ok());
        assert!(Pattern::parse("$OBJ.send($ARG)").is_ok());
        for pattern in [
            "",
            "let x = 1",
            "$lower",
            "foo::$X()",
            "{ call($X) }",
            "m!($X)",
            "x.$FIELD",
        ] {
            assert!(Pattern::parse(pattern).is_err(), "{pattern}");
        }
    }
    #[test]
    fn captures_compare_nodes_not_raw_whitespace_or_binding() {
        let stopped = std::sync::atomic::AtomicBool::new(false);
        let control = ObservationControl {
            stopped: &stopped,
            cancelled: &|| false,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
        };
        let pattern = Pattern::parse("send($X, $X)").unwrap();
        for (text, expected) in [
            ("send(a + b, a+b)", true),
            ("send(資料, 資料)", true),
            ("send(a, b)", false),
            ("\"send(a,a)\"", false),
        ] {
            let expression: Expr = syn::parse_str(text).unwrap();
            let mut captures = BTreeMap::new();
            assert_eq!(
                matches_expression(
                    &pattern.expression,
                    &expression,
                    &pattern.text,
                    text,
                    &mut captures,
                    &control,
                ),
                expected,
                "{text}"
            );
            for capture in captures.values() {
                assert!(text.get(capture.range.clone()).is_some());
            }
        }
    }
    #[test]
    fn parser_preflight_bounds_flat_unary_try_and_postfix_chains() {
        for expression in [
            format!("{}value", "!".repeat(300)),
            format!("value{}", "?".repeat(300)),
            format!("value{}", ".method()".repeat(300)),
            format!("value{}", "+value".repeat(300)),
        ] {
            assert!(lexical_bound(&expression).is_err());
            assert!(Pattern::parse(&expression).is_err());
        }
        assert!(lexical_bound("fn actual() { send(value,value); send(other,other); }").is_ok());
    }
    #[test]
    fn cancellation_interrupts_recursive_node_matching() {
        let pattern = Pattern::parse("send($X,$X)").unwrap();
        let text = "send(value,value)";
        let expression: Expr = syn::parse_str(text).unwrap();
        let stopped = std::sync::atomic::AtomicBool::new(false);
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, std::sync::atomic::Ordering::AcqRel) >= 3;
        let control = ObservationControl {
            stopped: &stopped,
            cancelled: &cancel,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(30),
        };
        assert!(!matches_expression(
            &pattern.expression,
            &expression,
            &pattern.text,
            text,
            &mut BTreeMap::new(),
            &control
        ));
        assert_eq!(checks.load(std::sync::atomic::Ordering::Acquire), 4);
    }
}
