#!/usr/bin/env python3
"""XC07 actual-binary PTY journeys at 120x36 and 60x18.

Uses existing isolated native/loopback and TM helpers; no builds, SQL writes,
paid/live provider calls or fabricated instruction sources. Records suggestions,
exact observed variants, neutral teaching, 32-entry library/33rd replacement,
explicit quick assignment, ordinary use review and restart.
"""
import smoke_sizes
import argparse
import json
from pathlib import Path
import platform
import tempfile
import traceback

from moves_smoke import Journey, Peer, save, digest, wait_for, HOME, DELETE, DOWN, RIGHT, ESC, PAGE
from native_connection_smoke import decoded_payload
from tm_smoke import definition


class SimilarMoves(Journey):
    def review_text(self):
        seen = []
        for index in range(256):
            seen.append(self.session.display())
            self.capture("variant-review-%03d" % index)
            if "[Stay]" in self.session.display():
                return "\n".join(seen)
            self.session.send(PAGE)
        raise AssertionError("complete teach review never became confirmable")

    def fill_review(self, record, expected):
        before = self.effects()
        self.menu()
        self.session.send("\r")
        wait_for(self.session, "Fill move parameters / JSON string array")
        self.replace_editor("x" * 256, '["src/c.rs"]')
        self.session.send("\r")
        wait_for(self.session, "Fill request parameters")
        observed = self.review_text()
        literal = record["proposal"]["steps"]["Request"]["instruction"]
        slot = record["proposal"]["request_parameters"]["slots"][0]
        assert "Move #%d revision %d" % (record["id"], record["revision"]) in observed
        assert "Exact saved literal:" in observed and literal in observed
        assert "bytes %d..%d" % (slot["start"], slot["end"]) in observed
        assert 'default "%s"' % slot["default"] in observed
        assert '["src/c.rs"]' in observed and "Complete substituted instruction:" in observed
        assert expected in observed
        self.quiet_assert(lambda: self.effects() == before, "parameter entry/review dispatched work")
        return observed

    def import_name(self, path, name, replace=None, commit=True):
        self.workspace()
        self.session.command("move-import")
        wait_for(self.session, "Import TM: literal")
        self.session.paste(str(path))
        self.session.send("\r")
        wait_for(self.session, "Import TM / choose")
        self.session.send("\r")
        wait_for(self.session, "Move name")
        self.move_ready()
        self.replace_editor("x" * 64, name)
        self.session.send("\r")
        if replace is not None:
            wait_for(self.session, "Give up learning")
            if not commit:
                self.capture("library-full-neutral-forget-review")
                self.session.send("\r")
                self.workspace()
                return
            self.session.send(DOWN * (replace + 1) + "\r")
        wait_for(self.session, "Confirm local change")
        self.review_text()
        self.session.send("y" if commit else "\r")
        self.until(lambda: any(row["proposal"]["name"] == name for row in self.moves()), "TM learning acknowledged")
        self.move_ready()
        self.workspace()

    def journey(self):
        self.connect()
        self.form("espeon")
        first, second = "fix the bug in src/a.rs", "Fix bug in src/b.rs"
        for index, text in enumerate((first, second, first)):
            self.send_request(text, "XC07-ACK-%d" % index)
        self.write_request("")
        before, effects = self.state()["moves"], self.effects()
        self.session.command("learn-move")
        wait_for(self.session, "Move name")
        self.move_ready()
        suggestion = self.session.display().splitlines()[self.session.screen.cursor.y].strip(" │|┆")
        self.replace_editor("x" * 64, "Parameterized fix")
        self.session.send("\r")
        wait_for(self.session, "Confirm local change")
        observed = self.review_text()
        assert "src/a.rs" in observed and "src/b.rs" in observed and "parameters" in observed
        assert self.state()["moves"] == before and self.effects() == effects
        self.session.send("\r")
        self.workspace()
        assert self.state()["moves"] == before, "neutral teaching learned a move"
        self.session.command("learn-move")
        wait_for(self.session, "Move name")
        self.move_ready()
        self.replace_editor("x" * 64, "Parameterized fix")
        self.session.send("\r")
        wait_for(self.session, "Confirm local change")
        self.review_text()
        self.session.send("y")
        self.until(lambda: len(self.moves()) == 1, "reviewed variant saved")
        learned = self.moves()[0]
        assert learned["proposal"]["steps"]["Request"]["instruction"] in (first, second)
        assert self.effects() == effects, "learning dispatched work"
        self.move_ready()
        self.workspace()
        self.evidence.append({"suggestion": suggestion, "observed_variants": [first, second],
                              "saved_exact_instruction": learned["proposal"]["steps"]["Request"]["instruction"],
                              "grouping": "three actual loopback dispatches, two path variants; no model grouping"})
        template = learned["proposal"]["request_parameters"]
        assert len(template["slots"]) == 1 and len(template["variants"]) == 2
        literal = learned["proposal"]["steps"]["Request"]["instruction"]
        slot = template["slots"][0]
        raw = literal.encode("utf-8")
        expected = (raw[:slot["start"]] + b"src/c.rs" + raw[slot["end"]:]).decode("utf-8")
        # A full slot review never overwrites another authored Request.
        self.write_request("Keep my exact other draft")
        self.session.send(HOME + RIGHT * 4 + "\x13")
        def other_draft():
            return next(draft for draft in self.task()["partner_drafts"][str(self.task()["lead_partner"])]["drafts"]
                        if draft["field"] == "Request")
        self.until(lambda: other_draft()["cursor"] == 4, "authored Request caret retained")
        retained_draft = other_draft()
        before_fill = self.effects()
        self.fill_review(learned, expected)
        self.session.send("y")
        wait_for(self.session, "Request draft retained")
        self.capture("parameter-use-preserves-other-draft")
        self.workspace()
        assert other_draft() == retained_draft, "parameter use changed the other Request or caret"
        assert self.effects() == before_fill
        self.write_request("")
        self.fill_review(learned, expected)
        self.session.send("\r")
        self.workspace()
        assert self.effects() == before_fill and self.moves()[0] == learned
        self.fill_review(learned, expected)
        self.session.send("y")
        wait_for(self.session, "Run once")
        self.capture("filled-request-still-needs-normal-outgoing-consent")
        assert self.effects() == before_fill
        self.session.send("d")
        receipt = self.pages("parameterized-request-full-payload-receipt", count=256)
        assert expected in receipt, "ordinary receipt omitted the complete substituted Request"
        self.session.send("d")
        wait_for(self.session, "Run once")
        self.quiet_assert(lambda: self.effects() == before_fill, "receipt inspection dispatched work")
        self.session.send("\r")
        self.workspace()
        self.quiet_assert(lambda: self.effects() == before_fill, "neutral outgoing review dispatched work")
        self.write_request("")
        self.fill_review(learned, expected)
        self.session.send("y")
        wait_for(self.session, "Run once")
        assert self.effects() == before_fill
        posts = len(self.peer.snapshot("POST"))
        self.peer.plans.put(("complete", "XC07-PARAMETERIZED-USE-ACK"))
        self.session.send(RIGHT + "\r")
        self.until(lambda: len(self.peer.snapshot("POST")) == posts + 1, "reviewed substituted request actually submitted")
        wait_for(self.session, "XC07-PARAMETERIZED-USE-ACK")
        self.until(lambda: not self.state()["active_work"] and "Storage: saved" in self.session.display(), "parameterized use settled")
        _, messages = decoded_payload("chat_completions", self.peer.snapshot("POST")[-1]["body"])
        assert messages[-1] == ("user", expected), messages[-1]
        assert sum(message == ("user", expected) for message in messages) == 1, messages
        self.quiet_assert(lambda: len(self.peer.snapshot("POST")) == posts + 1,
                          "one explicit Run submitted the substituted instruction more than once")
        assert self.moves()[0]["proposal"] == learned["proposal"], "using parameters rewrote saved text/template"
        self.evidence.append({"reviewed_slot_values": ["src/c.rs"], "actual_submitted_instruction": expected,
                              "only_slot_bytes_changed": True, "normal_outgoing_consent_required": True})
        self.workspace()
        self.write_request("")
        # Parameterized TMs export as strict TM2 and retain slots on reviewed import.
        tm2 = self.project / "parameterized-v2.toml"
        self.menu(details=True)
        self.session.send("e")
        wait_for(self.session, "Export TM: NEW")
        self.session.paste(str(tm2))
        self.session.send("\r")
        wait_for(self.session, "Create-only TM export")
        self.session.send("y")
        self.until(tm2.is_file, "parameterized TM2 published")
        self.move_ready()
        self.workspace()
        assert "schema_version = 2" in tm2.read_text(encoding="utf-8")
        self.import_name(tm2, "Imported parameters")
        imported = next(row for row in self.moves() if row["proposal"]["name"] == "Imported parameters")
        assert imported["proposal"]["request_parameters"] == template
        old_tm = self.project / "old-schema-1.toml"
        old_tm.write_text(definition(4), encoding="utf-8")
        for index in range(3, 33):
            self.import_name(old_tm, "Library %02d" % index)
        assert len(self.moves()) == 32
        self.session.command("move-library")
        wait_for(self.session, "Library /32")
        self.capture("library-32-list")
        self.workspace()
        full = self.state()["moves"]
        self.import_name(old_tm, "Library replacement", replace=31, commit=False)
        assert self.state()["moves"] == full, "neutral full-library choice evicted a move"
        self.import_name(old_tm, "Library replacement", replace=31)
        assert len(self.moves()) == 32
        chosen = next(row for row in self.moves() if row["proposal"]["name"] == "Library replacement")
        self.session.command("move-library")
        wait_for(self.session, "Library /32")
        self.session.send(DOWN * 31 + "d")
        wait_for(self.session, "Assign quick slot")
        self.capture("library-selected-explicit-quick-assignment")
        before_effects = self.effects()
        self.session.send("1")
        self.until(lambda: next(row for row in self.moves() if row["id"] == chosen["id"])["proposal"].get("quick_slot") == 0,
                   "explicit quick slot assignment committed")
        self.move_ready()
        assert self.effects() == before_effects
        self.capture("four-quick-moves-after-assignment")
        quick = [row for row in self.moves() if isinstance(row["proposal"].get("quick_slot"), int)]
        assert len(quick) == 4
        self.workspace()
        self.restart()
        self.connect()
        assert self.effects() == before_effects, "explicit reconnection replayed work"
        retained = next(row for row in self.moves() if row["id"] == chosen["id"])
        assert retained["proposal"]["quick_slot"] == 0
        assert len(self.moves()) == 32
        assert next(row for row in self.moves() if row["id"] == learned["id"])["proposal"]["request_parameters"] == template
        assert next(row for row in self.moves() if row["id"] == imported["id"])["proposal"]["request_parameters"] == template
        self.session.command("move-library")
        wait_for(self.session, "Library /32")
        self.session.send(DOWN * 31 + "\r")
        wait_for(self.session, "Run once")
        self.capture("library-use-ordinary-outgoing-review-after-restart")
        assert self.effects() == before_effects, "library use sent without normal consent"
        self.session.send("d")
        receipt = self.pages("library-request-full-payload-receipt-after-restart", count=256)
        assert chosen["proposal"]["steps"]["Request"]["instruction"] in receipt
        self.session.send("d")
        wait_for(self.session, "Run once")
        self.quiet_assert(lambda: self.effects() == before_effects,
                          "restarted library selection or receipt inspection dispatched work")
        self.session.command("cancel")
        self.workspace()
        self.quiet_assert(lambda: self.effects() == before_effects, "cancelling library use dispatched work")
        self.quit()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    assert binary.is_file()
    output.mkdir(parents=True, exist_ok=False)
    original_hash = digest(binary)
    result = {"status": "failed", "binary": str(binary), "sha256": original_hash,
              "host": platform.platform(), "scope": "Actual PTY and loopback only; no paid/live provider; no SQL writes", "sizes": []}
    try:
        for size, ascii_mode in smoke_sizes.selected((((120, 36), False), ((60, 18), True))):
            directory = output / ("%dx%d" % size)
            row = {"size": list(size), "status": "failed"}
            result["sizes"].append(row)
            driver = None
            with tempfile.TemporaryDirectory(prefix="xc07-moves-") as root, Peer("chat_completions", "unused-keyless", True) as peer:
                try:
                    driver = SimilarMoves(binary, directory, Path(root).resolve(), size, ascii_mode, {}, peer)
                    driver.journey()
                    peer.assert_clean()
                    row["status"] = "passed"
                except Exception as error:
                    row.update(error=str(error), traceback=traceback.format_exc())
                    if driver:
                        driver.capture("failure")
                finally:
                    if driver:
                        driver.close()
                    save(directory / "http-requests.json", peer.snapshot())
            assert digest(binary) == original_hash, "immutable binary changed"
        result["status"] = "passed" if all(row["status"] == "passed" for row in result["sizes"]) else "failed"
    except Exception as error:
        result.update(error=str(error), traceback=traceback.format_exc())
    finally:
        save(output / "results.json", result)
    print(json.dumps({"status": result["status"], "results": str(output / "results.json")}))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
