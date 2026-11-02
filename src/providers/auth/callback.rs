//! Fixed, private-data-free pages for Eevee-owned loopback callbacks.
use std::io::{self, Write};

#[derive(Clone, Copy)]
pub(super) enum State {
    Received,
    Denied,
    Invalid,
    NotFound,
    MethodNotAllowed,
}

struct Page {
    status: &'static str,
    eyebrow: &'static str,
    title: &'static str,
    detail: &'static str,
    guidance: &'static str,
}

impl State {
    fn page(self) -> Page {
        match self {
            Self::Received => Page {
                status: "200 OK",
                eyebrow: "RESPONSE RECEIVED",
                title: "Back to your workbench.",
                detail: "Your browser has handed the authorization response to Eevee. This is not a connection confirmation: the terminal still needs to validate the response and finish the credential exchange.",
                guidance: "Switch to the Eevee terminal and wait for its connection result before continuing. You can close this browser tab; keep the terminal open.",
            },
            Self::Denied => Page {
                status: "200 OK",
                eyebrow: "AUTHORIZATION NOT GRANTED",
                title: "No connection was made.",
                detail: "The provider returned a refusal or authorization error. Eevee has not established a connection from this response.",
                guidance: "Return to the Eevee terminal to review the result. If you want to try again, start a new sign-in there. You can close this tab.",
            },
            Self::Invalid => Page {
                status: "400 Bad Request",
                eyebrow: "RESPONSE NOT ACCEPTED",
                title: "This response does not match.",
                detail: "Eevee could not accept this callback. Your current sign-in attempt is still open in the terminal; this page has not completed or cancelled it.",
                guidance: "Return to the Eevee terminal and follow the current sign-in instructions. Use only the browser flow you opened for that attempt; do not share or reuse callback URLs.",
            },
            Self::NotFound => Page {
                status: "404 Not Found",
                eyebrow: "PAGE NOT FOUND",
                title: "A little off the trail.",
                detail: "This local page is not an Eevee sign-in callback. No authorization response was accepted, and the current terminal attempt is still open.",
                guidance: "Return to the Eevee terminal for the current sign-in instructions. You can close this tab.",
            },
            Self::MethodNotAllowed => Page {
                status: "405 Method Not Allowed",
                eyebrow: "REQUEST NOT ACCEPTED",
                title: "Not the expected handoff.",
                detail: "This local sign-in page accepts browser navigation only. No authorization response was accepted, and the current terminal attempt is still open.",
                guidance: "Return to the Eevee terminal and follow the current sign-in instructions. You can close this tab.",
            },
        }
    }
}

const START: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<meta name="referrer" content="no-referrer">
<title>Eevee · Sign-in handoff</title>
<style>
:root{color-scheme:light dark;--paper:#f5f0e6;--card:#fffaf1;--ink:#342d25;--muted:#655d51;--line:#d8ccba;--leaf:#435d43;--soft:#e8eddf;--shadow:#66503918}
*{box-sizing:border-box}html{min-height:100%;scroll-behavior:auto}body{margin:0;background:var(--paper);color:var(--ink);font:1rem/1.65 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}body:before{content:"";position:fixed;inset:0;pointer-events:none;background-image:radial-gradient(var(--line) .7px,transparent .7px);background-size:20px 20px;opacity:.48}main{position:relative;max-width:1080px;margin:auto;padding:clamp(24px,5vw,64px)}.brand{display:flex;align-items:center;gap:12px;font-weight:750;letter-spacing:.02em}.mark{display:grid;place-items:center;width:36px;height:36px;background:var(--ink);color:var(--card);border-radius:10px;font-size:20px}.brand small{display:block;color:var(--muted);font-size:.72rem;font-weight:500;letter-spacing:.13em;text-transform:uppercase}.card{margin-top:36px;display:grid;grid-template-columns:minmax(0,1.5fr) minmax(220px,1fr);background:var(--card);border:1px solid var(--line);border-radius:24px;overflow:hidden;box-shadow:0 18px 65px var(--shadow)}.content{padding:clamp(26px,4vw,48px)}.eyebrow{display:flex;align-items:center;gap:9px;color:var(--leaf);font-size:.72rem;font-weight:750;letter-spacing:.13em}.eyebrow:before{content:"";width:8px;height:8px;flex-shrink:0;background:currentColor;border-radius:50%}h1{font-size:clamp(2rem,4.5vw,3.2rem);line-height:1.12;letter-spacing:-.045em;margin:22px 0}p{color:var(--muted);margin:0 0 24px;max-width:57ch}.action{display:inline-flex;gap:14px;align-items:center;background:var(--leaf);color:#fffdf5;border:2px solid transparent;border-radius:10px;padding:12px 16px;font-size:.88rem;font-weight:650;text-decoration:none;line-height:1.5}.action:hover{filter:brightness(1.12)}a:focus-visible{outline:3px solid var(--ink);outline-offset:5px}.art{display:flex;flex-direction:column;align-items:center;justify-content:center;padding:32px 20px;background:var(--soft);border-left:1px solid var(--line)}.art svg{width:100%;max-width:260px;height:auto}.art span{font:600 .68rem/1.5 ui-monospace,monospace;text-transform:uppercase;letter-spacing:.16em;color:var(--leaf);margin-top:16px}.guidance{margin-top:28px;padding:24px 0;border-top:1px solid var(--line)}.guidance:focus{outline:2px solid var(--leaf);outline-offset:8px;border-radius:4px}h2{font-size:1rem;letter-spacing:-.01em;margin:0 0 10px}.guidance p{margin:0;max-width:76ch;font-size:.92rem}footer{display:flex;justify-content:space-between;gap:14px;flex-wrap:wrap;color:var(--muted);font-size:.75rem;margin-top:14px}footer span:last-child{font-family:ui-monospace,monospace}
@media(max-width:640px){main{padding:22px 18px}.card{grid-template-columns:minmax(0,1fr);margin-top:24px;border-radius:18px}.art{border-left:0;border-top:1px solid var(--line);padding:18px}.art svg{max-width:160px}.art span{margin-top:0}.content{padding:28px 24px}.action{width:100%;justify-content:space-between}h1{font-size:2.25rem}.guidance{margin-top:22px}}
@media(prefers-color-scheme:dark){:root{--paper:#211f1b;--card:#2c2923;--ink:#f2e9d7;--muted:#c5bcaa;--line:#514b3e;--leaf:#c0d4a7;--soft:#343c2e;--shadow:#0004}.action{background:#c0d4a7;color:#232c1d}.mark{background:#ddc6a1;color:#29231c}}
</style>
</head>
<body><main>
<header class="brand"><span class="mark" aria-hidden="true">e.</span><div>Eevee<small>Your agent workbench</small></div></header>
<article class="card" aria-labelledby="status-title"><div class="content"><div class="eyebrow">"##;
const TITLE: &str = "</div><h1 id=\"status-title\">";
const DETAIL: &str = "</h1><p>";
const AFTER_DETAIL: &str = r##"</p><a class="action" href="#terminal">Show terminal instructions <span aria-hidden="true">↓</span></a></div>
<div class="art" aria-hidden="true">
<svg viewBox="0 0 260 260" xmlns="http://www.w3.org/2000/svg" focusable="false">
<ellipse cx="130" cy="233" rx="88" ry="9" fill="#66764f" opacity=".2"/>
<path d="M185 206 211 187 230 153 240 157 237 197 217 225 182 227Z" fill="#ae7952"/>
<path d="m230 153 10 4-3 40-15-4-11-6Z" fill="#eddbb4"/>
<path d="m87 158 86-2 18 65-28 11-23-14-25 14-39-12Z" fill="#b48258"/>
<path d="m82 135-20-65 4-42 35 34 19 69m26-2 24-66 30-31 2 42-22 67" fill="#ae7952"/>
<path d="M78 60 85 78 99 118 85 111 73 77Zm101 11-15 43 13-7 14-38-1-15Z" fill="#594331"/>
<path d="m70 117 35-23 47 1 39 23 5 39-31 31-65-1-33-31Z" fill="#c79767"/>
<path d="m72 163 15-10 13 18 17-9 15 14 16-14 16 10 16-18 16 11-12 23-16 4-6 21-23-11-11 14-16-14-21 9-5-21-17-7Z" fill="#f3e3be"/>
<path d="M99 128h10v13H99Zm54 0h10v13h-10Z" fill="#342d25"/>
<path d="m124 146 14 0-7 7Z" fill="#43352a"/>
<path d="m124 158 7 4 7-4" fill="none" stroke="#725039" stroke-width="3" stroke-linecap="round"/>
<path d="M96 221h18m35 0h18" stroke="#725039" stroke-width="4" stroke-linecap="round"/>
<path d="M39 132h16m-8-8v16m160-36h12m-6-6v12" stroke="#839267" stroke-width="3" stroke-linecap="square"/>
<path d="M43 213h14m-7-7v14" stroke="#a5916c" stroke-width="3"/>
</svg><span>A quiet place to land</span></div></article>
<section id="terminal" class="guidance" tabindex="-1" aria-labelledby="terminal-title"><h2 id="terminal-title">The next step is in Eevee.</h2><p>"##;
const END: &str = r##"</p></section>
<footer><span>Local browser handoff · No account details shown</span><span>eevee / workbench</span></footer>
</main></body></html>
"##;

/// The caller owns socket deadlines and callback acceptance. A browser page write
/// must never change the authorization result; callers deliberately ignore errors.
pub(super) fn respond(stream: &mut impl Write, state: State) -> io::Result<()> {
    let page = state.page();
    let parts = [
        START,
        page.eyebrow,
        TITLE,
        page.title,
        DETAIL,
        page.detail,
        AFTER_DETAIL,
        page.guidance,
        END,
    ];
    let length: usize = parts.iter().map(|part| part.len()).sum();
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {length}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; sandbox\r\nX-Frame-Options: DENY\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        page.status,
    )?;
    for part in parts {
        stream.write_all(part.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_pages_delimit_utf8_and_protect_every_response() {
        for (state, status) in [
            (State::Received, "200 OK"),
            (State::Denied, "200 OK"),
            (State::Invalid, "400 Bad Request"),
            (State::NotFound, "404 Not Found"),
            (State::MethodNotAllowed, "405 Method Not Allowed"),
        ] {
            let mut bytes = Vec::new();
            respond(&mut bytes, state).unwrap();
            let response = String::from_utf8(bytes).unwrap();
            let (head, body) = response.split_once("\r\n\r\n").unwrap();
            assert!(head.starts_with(&format!("HTTP/1.1 {status}\r\n")));
            let headers: std::collections::HashMap<_, _> = head
                .split("\r\n")
                .skip(1)
                .map(|line| line.split_once(": ").unwrap())
                .collect();
            assert_eq!(headers["Content-Type"], "text/html; charset=utf-8");
            assert_eq!(
                headers["Content-Length"].parse::<usize>().unwrap(),
                body.len()
            );
            assert_eq!(headers["Cache-Control"], "no-store");
            assert_eq!(headers["Referrer-Policy"], "no-referrer");
            assert_eq!(headers["X-Frame-Options"], "DENY");
            assert_eq!(headers["X-Content-Type-Options"], "nosniff");
            let csp = headers["Content-Security-Policy"];
            for directive in [
                "default-src 'none'",
                "base-uri 'none'",
                "form-action 'none'",
                "frame-ancestors 'none'",
                "sandbox",
            ] {
                assert!(csp.split(';').any(|part| part.trim() == directive));
            }
        }
    }
}
