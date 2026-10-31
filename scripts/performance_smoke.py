#!/usr/bin/env python3
"""Measure Q02 against an already-built native ev; never build or launch a model.

Python 3.11+; optional dependencies (install explicitly in the invoking environment):
  python -m pip install pyte==0.8.2 wcwidth==0.2.13 psutil==7.0.0

Uses terminal_smoke's actual POSIX PTY / Windows ConPTY and workbench helpers.
All project writes and approved commands run in a disposable directory outside
of the checkout. Evidence goes under --output. Provider PIDs are observe-only;
this program never starts, submits to, or terminates an external provider.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack, closing
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import platform
import queue
import shutil
import sqlite3
import sys
import tempfile
import threading
import time
import traceback

from terminal_smoke import (ESC, RIGHT, WINDOWS, Session, confirm_exit, command_version,
                            host_evidence, run_owned, save_json, terminal_environment)
from workbench_smoke import wait_for

MIB = 1024 * 1024
IDLE_SECONDS = 60
STARTUP_SAMPLES = 30
GOAL = "Q02 disposable local measurement"
SCOPES = ("core", "local_commands", "app_helpers", "sdk_sidecar", "external_agent", "measurement_harness")

# Executed only by the real approved local runner, never imported by the harness.
WORKLOAD = r'''import json
import os
from pathlib import Path
import sys
import time

mode, directory = sys.argv[1:]
root = Path(directory)
started = time.monotonic()
written = 0
iteration = 0

def evidence(done=False):
    value = {"pid": os.getpid(), "cwd": os.getcwd(), "mode": mode,
             "bytes_written": written, "iterations": iteration,
             "elapsed_seconds": time.monotonic() - started, "done": done}
    if done and os.name != "nt":
        import resource
        usage = resource.getrusage(resource.RUSAGE_SELF)
        value["native_self_cpu_seconds"] = usage.ru_utime + usage.ru_stime
        value["native_peak_rss_bytes"] = int(usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024))
    temporary = root / "state.tmp"
    temporary.write_text(json.dumps(value), encoding="utf-8")
    temporary.replace(root / "state.json")

evidence()
if mode == "stream":
    while not (root / "release").exists():
        if time.monotonic() - started > 120:
            raise RuntimeError("stream workload exceeded its owned 120-second deadline")
        line = ("PERF-STREAM-%06d " % iteration).ljust(127, ".") + "\n"
        sys.stdout.buffer.write(line.encode("ascii"))
        sys.stdout.buffer.flush()
        written += len(line)
        iteration += 1
        evidence()
        time.sleep(0.01)
elif mode == "output":
    block = b"x" * 4095 + b"\n"
    for iteration in range(1, 2561):
        sys.stdout.buffer.write(block)
        sys.stdout.buffer.flush()
        written += len(block)
        time.sleep(0.001)
else:
    raise RuntimeError("unknown workload")
evidence(True)
'''


def digest(path):
    with Path(path).open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def direct_input(session, keys):
    """No Session.send(): its fixed 250 ms drain is not application latency."""
    session.events.append({"input": repr(keys), "method": "direct write", "monotonic_ns": time.monotonic_ns()})
    if WINDOWS:
        session.process.write(keys)
    else:
        pending = keys.encode("utf-8")
        while pending:
            count = os.write(session.master, pending)
            if count <= 0:
                raise RuntimeError("terminal input stopped accepting bytes")
            pending = pending[count:]


def drain_one(session, timeout):
    try:
        data = session.data.get(timeout=max(0, timeout))
    except queue.Empty:
        return False
    if data is None:
        session.eof = True
        return False
    if isinstance(data, dict):
        session.events.append(data)
        return False
    text = session.decoder.decode(data)
    session.text += text
    session.stream.feed(text)
    if len(session.text) > 16 * MIB:
        raise RuntimeError("terminal evidence exceeded Session's 16 MiB bound")
    return True


def visible(session, predicate, timeout=10):
    """Stop at the first decoded frame satisfying the visible-state predicate."""
    deadline = time.monotonic() + timeout
    while True:
        if predicate(session.display()):
            return time.monotonic_ns()
        if not session.alive() or session.eof:
            raise RuntimeError("application exited before the expected visible response")
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("visible response deadline expired:\n" + session.display())
        drain_one(session, min(remaining, 0.02))


def raw_latency(session, keys, predicate):
    if predicate(session.display()):
        raise RuntimeError("latency predicate already visible before input")
    started = time.monotonic_ns()
    direct_input(session, keys)
    ended = visible(session, predicate)
    return (ended - started) / 1_000_000


def distribution(values, target=None):
    ordered = sorted(values)
    result = {"sample_count": len(values), "samples_ms": values,
              "minimum_ms": ordered[0], "median_ms": ordered[(len(ordered) - 1) // 2],
              "p95_ms": ordered[math.ceil(len(ordered) * 0.95) - 1], "maximum_ms": ordered[-1],
              "percentile_method": "nearest rank ceil(0.95*n), no interpolation"}
    if target is not None:
        result.update(target_p95_ms=target, target_met=result["p95_ms"] <= target)
    return result


class Resources:
    """Observed process trees, identity-bound by PID plus creation time.

    Sampling cannot see a process that starts and exits between samples. It is
    explicitly a lower bound, not an assertion that transient helpers are free.
    The harness is reported separately and in an inclusive aggregate.
    """
    def __init__(self, psutil, binary, directory, providers, interval, session=None):
        self.psutil, self.binary, self.directory = psutil, binary, directory
        self.providers, self.interval, self.session = providers, interval, session
        self.launch_window = session is None
        self.records, self.owned = {}, {}
        self.errors = set()
        self.peaks = {scope: 0 for scope in (*SCOPES, "application_total", "including_measurement_harness")}
        self.samples = 0
        self.started = time.monotonic()
        self.wall_started = time.time()
        self.stop_event = threading.Event()
        directory.mkdir(parents=True, exist_ok=True)
        self.file = (directory / "resources.jsonl").open("w", encoding="utf-8")
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def _tree(self, process, scope, found, owned=False):
        found[process.pid] = (process, scope, owned)
        try:
            for child in process.children(recursive=True):
                found.setdefault(child.pid, (child, scope, owned))
        except self.psutil.Error as error:
            self.errors.add(type(error).__name__ + ": tree pid=" + str(process.pid))

    def _sample(self):
        found = {}
        for scope, process in self.providers:
            self._tree(process, scope, found)
        session = self.session
        if session is not None and session.alive():
            try:
                root = self.psutil.Process(session.process.pid)
                self._tree(root, "app_helpers", found, owned=True)
                if WINDOWS:
                    found[root.pid] = (root, "measurement_harness", True)
                else:
                    found[root.pid] = (root, "core", True)
            except self.psutil.Error:
                pass
        # Retain already-observed owned children if reparented during shutdown.
        for identity, process in list(self.owned.items()):
            found.setdefault(process.pid, (process, self.records[identity]["scope"], True))
        harness = self.psutil.Process(os.getpid())
        found[harness.pid] = (harness, "measurement_harness", False)
        rss = {scope: 0 for scope in SCOPES}
        for process, scope, owned in found.values():
            try:
                if not process.is_running():
                    continue
                with process.oneshot():
                    created = process.create_time()
                    identity = (process.pid, created)
                    # A completed child can lose its command line before it is
                    # reaped. Keep its already observed, identity-bound scope.
                    if owned and scope == "app_helpers" and identity in self.records:
                        scope = self.records[identity]["scope"]
                    if owned and scope == "app_helpers":
                        executable = process.exe()
                        if os.path.normcase(executable) == os.path.normcase(str(self.binary)):
                            scope = "core"
                        elif any("q02_workload.py" in part for part in process.cmdline()):
                            scope = "local_commands"
                    cpu = process.cpu_times()
                    seconds = cpu.user + cpu.system
                    waited_cpu = (cpu.children_user + cpu.children_system) if hasattr(cpu, "children_user") else None
                    memory = process.memory_info().rss
                    name = process.name()
                if identity not in self.records:
                    # Count all CPU of a newly launched process, but never its
                    # pre-window lifetime if it was already running.
                    new_process = (owned and self.launch_window) or created >= self.wall_started
                    baseline = 0.0 if new_process else seconds
                    waited_baseline = (0.0 if new_process else waited_cpu) if waited_cpu is not None else None
                    self.records[identity] = {"pid": process.pid, "created_epoch_seconds": created,
                                              "name": name, "scope": scope, "cpu_baseline_seconds": baseline,
                                              "waited_children_cpu_baseline_seconds": waited_baseline,
                                              "waited_children_cpu_last_seconds": waited_cpu,
                                              "cpu_last_seconds": seconds, "sampled_peak_rss_bytes": 0,
                                              "first_seen_seconds": time.monotonic() - self.started,
                                              "sample_count": 0}
                record = self.records[identity]
                record["scope"] = scope
                record["cpu_last_seconds"] = max(record["cpu_last_seconds"], seconds)
                record["waited_children_cpu_last_seconds"] = waited_cpu
                record["sampled_peak_rss_bytes"] = max(record["sampled_peak_rss_bytes"], memory)
                record["last_seen_seconds"] = time.monotonic() - self.started
                record["sample_count"] += 1
                rss[scope] += memory
                if owned:
                    self.owned[identity] = process
            except (self.psutil.NoSuchProcess, self.psutil.ZombieProcess):
                continue
            except self.psutil.Error as error:
                self.errors.add(type(error).__name__ + ": read pid=" + str(process.pid))
        rss["application_total"] = sum(value for key, value in rss.items() if key != "measurement_harness")
        rss["including_measurement_harness"] = rss["application_total"] + rss["measurement_harness"]
        for scope, value in rss.items():
            self.peaks[scope] = max(self.peaks[scope], value)
        self.samples += 1
        self.file.write(json.dumps({"elapsed_seconds": time.monotonic() - self.started, "rss_bytes": rss}) + "\n")

    def _run(self):
        try:
            while not self.stop_event.is_set():
                self._sample()
                self.stop_event.wait(self.interval)
            self._sample()
        except Exception as error:
            self.errors.add("sampler failed: " + repr(error))

    def finish(self):
        self.stop_event.set()
        self.thread.join(timeout=5)
        if self.thread.is_alive():
            raise RuntimeError("resource sampler did not stop within five seconds")
        self.file.close()
        duration = time.monotonic() - self.started
        groups = {}
        for scope in SCOPES:
            records = [record for record in self.records.values() if record["scope"] == scope]
            cpu = sum(record["cpu_last_seconds"] - record["cpu_baseline_seconds"] for record in records)
            groups[scope] = {"status": "observed" if records else "not_observed",
                             "observed_cpu_seconds": cpu,
                             "cpu_percent_one_core": 100 * cpu / duration,
                             "sampled_peak_rss_bytes": self.peaks[scope], "processes": records}
            if scope in ("sdk_sidecar", "external_agent") and not records:
                groups[scope] = {"status": "unavailable", "reason": "no accessible explicitly supplied provider PID", "observed_cpu_seconds": None,
                                 "cpu_percent_one_core": None, "sampled_peak_rss_bytes": None, "processes": []}
        for name, scopes in (("application_total", SCOPES[:-1]), ("including_measurement_harness", SCOPES)):
            measured = [groups[scope] for scope in scopes if groups[scope]["status"] != "unavailable"]
            cpu = sum(group["observed_cpu_seconds"] for group in measured)
            groups[name] = {"status": "observed_scopes_only", "observed_cpu_seconds": cpu,
                            "cpu_percent_one_core": 100 * cpu / duration,
                            "sampled_peak_rss_bytes": self.peaks[name],
                            "included_scopes": [scope for scope in scopes if groups[scope]["status"] != "unavailable"],
                            "unavailable_scopes": [scope for scope in scopes if groups[scope]["status"] == "unavailable"]}
        core_waited = [record for record in groups["core"]["processes"]
                       if record["waited_children_cpu_last_seconds"] is not None]
        waited_total = sum(record["waited_children_cpu_last_seconds"] - record["waited_children_cpu_baseline_seconds"]
                           for record in core_waited) if core_waited else None
        return {"duration_seconds": duration, "sample_count": self.samples,
                "requested_sample_interval_seconds": self.interval, "scopes": groups,
                "errors": sorted(self.errors),
                "native_core_waited_children_cpu_seconds": waited_total,
                "waited_children_note": "POSIX kernel counter supplements transient-helper visibility; overlaps sampled descendant CPU and is NOT added again to the aggregate",
                "coverage": "sampled process trees; CPU is a lower bound for short-lived/exited helpers; RSS peaks are sampled, not kernel lifetime peaks",
                "rss_aggregation": "simultaneous RSS sum; shared pages may be counted more than once",
                "excluded": "terminal emulator/desktop compositor outside native PTY/ConPTY; inaccessible OS console-host processes; processes wholly between samples",
                "provider_scope": "explicit PID and observed descendants during this window; no model request attribution or token/cost claim"}

    def cleanup_owned(self):
        """Only processes descended from our Session; never provider PID inputs."""
        survivors = []
        for process in self.owned.values():
            try:
                if process.is_running():
                    process.terminate()
                    survivors.append(process)
            except self.psutil.NoSuchProcess:
                pass
        _, alive = self.psutil.wait_procs(survivors, timeout=2)
        for process in alive:
            try:
                process.kill()
            except self.psutil.NoSuchProcess:
                pass
        _, alive = self.psutil.wait_procs(alive, timeout=3)
        if alive:
            raise RuntimeError("owned process cleanup deadline exceeded: " + repr([process.pid for process in alive]))


def rows(data):
    database = data / "workbench.sqlite3"
    with closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=10)) as connection:
        executions = [{"record": json.loads(record), "output_name": name, "reserved": bool(reserved)}
                      for record, name, reserved in connection.execute("SELECT record,output_name,reserved FROM executions ORDER BY rowid")]
        providers = connection.execute("SELECT count(*) FROM provider_journal").fetchone()[0]
    return executions, providers


def await_run(session, data, timeout=40):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        executions, providers = rows(data)
        if providers:
            raise RuntimeError("local workload unexpectedly created a provider journal")
        if executions and not executions[-1]["reserved"]:
            run = executions[-1]
            record = run["record"]
            if record["state"] != "Succeeded" or record["tree"] != "OwnedTreeExited" or record["exit_code"] != 0:
                raise RuntimeError("local workload did not succeed with its owned tree exited: " + repr(record))
            wait_for(session, "Local command evidence; no command running", timeout=timeout)
            return run
        if not session.alive():
            raise RuntimeError("application exited before committing execution evidence")
        session.pump(0.02)
    raise TimeoutError("local execution journal did not finalize")


def quit_app(session):
    session.send("\x03")  # Preserve the invoking composer/drawer across a normal safe exit.
    confirm_exit(session)
    if session.wait_exit() != 0:
        raise RuntimeError("application exited unsuccessfully")
    session.close()


def prepare_command(session, fixture, mode, state):
    session.send(ESC)
    session.command("goal")
    session.paste(GOAL)
    session.send("\r")
    wait_for(session, "Storage: saved")
    session.command("jolteon")  # Jolteon owns command preparation.
    session.command("command-edit")
    wait_for(session, "Command / explicit JSON argv or shell")
    argv = [sys.executable, "-X", "utf8", "-u", str(fixture), mode, str(state)]
    session.paste(json.dumps(argv))
    session.send("\r")
    session.command("execute")
    wait_for(session, "Review one-shot execution permission")
    session.capture("neutral-approval")
    if (state / "state.json").exists():
        raise RuntimeError("local command ran before affirmative approval")
    return argv


def hardware(psutil):
    result = {"processor": platform.processor(), "logical_cpu_count": psutil.cpu_count(),
              "physical_cpu_count": psutil.cpu_count(logical=False),
              "physical_memory_bytes": psutil.virtual_memory().total}
    if sys.platform == "darwin":
        result["native_hardware"] = command_version(["/usr/sbin/sysctl", "-n", "hw.model", "machdep.cpu.brand_string", "hw.memsize", "hw.physicalcpu", "hw.logicalcpu"])
    elif sys.platform.startswith("linux"):
        cpuinfo = Path("/proc/cpuinfo")
        if cpuinfo.exists():
            result["cpu_model_lines"] = sorted({line for line in cpuinfo.read_text().splitlines()
                                                if line.startswith(("model name", "Hardware", "Model"))})
    elif WINDOWS:
        result["processor_identifier"] = os.environ.get("PROCESSOR_IDENTIFIER")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default="target/release/ev", help="already-built release binary; never built by this script")
    parser.add_argument("--output", required=True, help="new or empty evidence directory; existing files are never overwritten")
    parser.add_argument("--build-profile", default="release", help="caller-declared build profile; not inferred from executable bytes")
    parser.add_argument("--key-samples", type=int, default=60, help="at least 30 actual follow-toggle key samples during streaming (default: 60)")
    parser.add_argument("--sample-interval", type=float, default=0.02, help="process sampling interval in seconds, 0.005..0.1 (default: 0.02)")
    parser.add_argument("--spool-kib", type=int, default=1024, help="real local-runner spool bound; must be below 10240 KiB (default: 1024)")
    parser.add_argument("--sdk-pid", action="append", type=int, default=[], help="observe an already-running optional SDK sidecar and descendants, never launch/kill it")
    parser.add_argument("--agent-pid", action="append", type=int, default=[], help="observe an already-running external agent and descendants, never launch/kill it")
    args = parser.parse_args()
    if not 30 <= args.key_samples <= 300 or not 0.005 <= args.sample_interval <= 0.1 or not 1 <= args.spool_kib < 10240:
        parser.error("require 30..300 key samples, 0.005..0.1 sampling seconds, and 1..10239 spool KiB")
    try:
        import psutil
        for package, version in (("psutil", "7.0.0"), ("pyte", "0.8.2"), ("wcwidth", "0.2.13")):
            if importlib.metadata.version(package) != version:
                parser.error("measurement dependency must be pinned: " + package + "==" + version + "; see --help")
    except (ImportError, importlib.metadata.PackageNotFoundError):
        parser.error("missing optional measurement dependencies; see --help for explicit pinned installation")
    binary = Path(args.binary).resolve()
    if WINDOWS and not binary.exists():
        binary = binary.with_suffix(".exe")
    if not binary.is_file():
        parser.error("already-built native binary not found: " + str(binary))
    output = Path(args.output).resolve()
    if output.exists() and (not output.is_dir() or any(output.iterdir())):
        parser.error("choose a new or empty --output directory; existing evidence will not be overwritten")
    output.mkdir(parents=True, exist_ok=True)
    providers = []
    seen = set()
    for scope, pids in (("sdk_sidecar", args.sdk_pid), ("external_agent", args.agent_pid)):
        for pid in pids:
            if pid in seen or pid == os.getpid():
                parser.error("provider roots must be distinct and cannot be this harness")
            try:
                process = psutil.Process(pid)
                process.create_time()
                family = {pid, *(child.pid for child in process.children(recursive=True))}
                if family & seen or os.getpid() in family:
                    parser.error("provider roots must not overlap or contain the harness")
                providers.append((scope, process))
                seen.update(family)
            except psutil.Error as error:
                parser.error("provider PID is unavailable: " + str(error))
    report = {"schema_version": 1, "status": "failed", "q02_acceptance": "partial; provider/context gates are separate",
              "binary": {"path": str(binary), "sha256": digest(binary), "size_bytes": binary.stat().st_size,
                         "build_profile": args.build_profile, "profile_source": "caller declaration, not independently verified"},
              "method": {"terminal_size": [120, 36], "images": "off", "color": "mono", "network_or_model_submission": "none requested",
                         "latency": "monotonic clock before direct terminal write through first decoded visible response; includes PTY/ConPTY, reader scheduling and pyte parsing; no fixed pump delay",
                         "startup": "30 warm launches after one untimed seed; existing saved empty task composer, complete saved-state/footer and visible cursor; process creation and terminal setup included",
                         "idle": "60-second local shell observation; one-core CPU = CPU seconds / wall seconds * 100; not divided by CPU count",
                         "resource_scope": "core, approved workload, other app helpers, explicit optional provider roots, harness and simultaneous aggregates",
                         "boundedness": "finite workload and configured spool observed; sampled RSS cannot prove every internal queue bound or a lifetime peak"},
              "provider_usage_and_context": {"status": "unavailable", "reason": "no live model request is launched; supplied PIDs measure resources only",
                                             "tokens": None, "cost": None, "provider_added_context": "unknown", "savings_percent": None},
              "measurements": {}, "misses": [], "unavailable": ["live provider usage/cost/context and request-attributed resource acceptance", "GUI terminal/compositor resource costs", "kernel lifetime peak RSS for the core and transient helpers"]}
    sessions, monitors, releases = [], [], []
    active = None
    started = time.monotonic()
    cleanup_done = False

    def cleanup():
        nonlocal cleanup_done, active
        if cleanup_done:
            return
        cleanup_done = True
        cleanup_errors = []
        # Release and kill owned work before removing the disposable project,
        # including on Windows where open handles prevent directory removal.
        for release in releases:
            try:
                if release.parent.exists():
                    release.touch(exist_ok=True)
            except OSError as error:
                cleanup_errors.append(str(error))
        if active is not None:
            try:
                report["interrupted_resources"] = active.finish()
                active = None
            except Exception as error:
                cleanup_errors.append(str(error))
        for resource in monitors:
            try:
                resource.cleanup_owned()
            except Exception as error:
                cleanup_errors.append(str(error))
        for session in sessions:
            try:
                if not session.closed:
                    session.capture("final-or-failure")
                    session.close()
            except Exception as error:
                cleanup_errors.append(str(error))
        if cleanup_errors:
            report["cleanup_errors"] = cleanup_errors
            report["status"] = "failed"

    try:
        report["host"] = host_evidence()
        report["host"]["hardware"] = hardware(psutil)
        report["host"]["packages"]["psutil"] = importlib.metadata.version("psutil")
        with ExitStack() as lifetime:
            temporary = lifetime.enter_context(tempfile.TemporaryDirectory(prefix="ev-q02-"))
            lifetime.callback(cleanup)
            root = Path(temporary).resolve()
            project = root / "project"
            project.mkdir()
            home = root / "home"
            home.mkdir()
            environment = terminal_environment("mono")
            for key in list(environment):
                if key.startswith(("OPENAI_", "CODEX_", "ANTHROPIC_", "CLAUDE_", "GEMINI_", "GOOGLE_", "AWS_", "OPENCODE_", "GIT_")):
                    environment.pop(key, None)
            environment.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / "config"),
                               XDG_DATA_HOME=str(home / "data"), XDG_CACHE_HOME=str(home / "cache"),
                               APPDATA=str(home / "config"), LOCALAPPDATA=str(home / "cache"),
                               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
            fixture = root / "q02_workload.py"
            fixture.write_text(WORKLOAD, encoding="utf-8")
            (output / "q02_workload.py").write_text(WORKLOAD, encoding="utf-8")

            def launch(name, data, cwd=project):
                session = Session([str(binary), "--continue", "--data-dir", str(data), "--assets-dir", str(root / "assets"),
                                   "--images", "off", "--color", "mono", "--spool-kib", str(args.spool_kib)],
                                  cwd, environment, (120, 36), output / name)
                sessions.append(session)
                return session

            def monitor(name, session=None):
                resource = Resources(psutil, binary, output / name, providers, args.sample_interval, session)
                monitors.append(resource)
                return resource

            def ready(session):
                observed = visible(session, lambda text: "Storage: saved" in text
                                   and not session.screen.cursor.hidden
                                   and bool(session.screen.display[-1].strip()))
                # The low-latency reader bypasses Session.pump; capture terminal
                # modes while the process is alive, without adding a timed wait.
                session.pump(0)
                return observed

            data = root / "startup-data"
            seed = launch("warm-seed", data)
            ready(seed)
            seed.capture("saved-composer")
            quit_app(seed)
            startup, probes, startup_resources = [], [], []
            for index in range(STARTUP_SAMPLES):
                name = "startup-%02d" % (index + 1)
                active = monitor(name)
                before = time.monotonic_ns()
                session = launch(name, data)
                active.session = session
                after = ready(session)
                startup.append((after - before) / 1_000_000)
                session.capture("usable-saved-composer")
                empty_composer = session.display()
                probes.append(raw_latency(session, "x", lambda text: text != empty_composer and "x" in text))
                direct_input(session, "\x7f")
                visible(session, lambda text: text == empty_composer)
                startup_resources.append(active.finish())
                active = None
                quit_app(session)
            report["measurements"]["warm_startup"] = {**distribution(startup, 200), "usability_key_probe": distribution(probes),
                                                           "resources_per_launch": startup_resources}

            session = launch("idle", data)
            ready(session)
            session.pump(0, settle=True)
            session.capture("before")
            before_output = len(session.text)
            active = monitor("idle", session)
            idle_started = time.monotonic()
            while time.monotonic() - idle_started < IDLE_SECONDS:
                if not session.alive():
                    raise RuntimeError("application exited during idle window")
                drain_one(session, min(0.05, max(0, IDLE_SECONDS - (time.monotonic() - idle_started))))
            idle_duration = time.monotonic() - idle_started
            resources = active.finish()
            active = None
            core = resources["scopes"]["core"]
            if not core["processes"]:
                raise RuntimeError("core process resource scope was not observed")
            cpu = core["observed_cpu_seconds"] * 100 / idle_duration
            peak = core["sampled_peak_rss_bytes"]
            report["measurements"]["idle"] = {"duration_seconds": idle_duration, "resources": resources,
                "terminal_output_bytes": len(session.text[before_output:].encode("utf-8")),
                "cpu_percent_one_core": cpu, "cpu_target_less_than_percent": 1, "cpu_target_met": cpu < 1,
                "sampled_peak_rss_bytes": peak, "rss_target_bytes": 80 * MIB, "rss_target_met": peak <= 80 * MIB}
            session.capture("after")
            quit_app(session)

            for mode in ("stream", "output"):
                state = root / (mode + "-state")
                state.mkdir()
                release = state / "release"
                releases.append(release)
                data = root / (mode + "-data")
                session = launch(mode, data)
                ready(session)
                argv = prepare_command(session, fixture, mode, state)
                active = monitor(mode, session)
                workload_started = time.monotonic()
                direct_input(session, RIGHT + "\r")  # Explicit affirmative one-shot approval.
                visible(session, lambda text: "PERF-STREAM-" in text if mode == "stream" else "CAPTURE TRUNCATED" in text, timeout=40)
                if mode == "stream":
                    first = json.loads((state / "state.json").read_text())
                    values = []
                    for index in range(args.key_samples):
                        expected = "Follow paused" if index % 2 == 0 else "Follow ON"
                        direct_input(session, "/follow")
                        values.append(raw_latency(session, "\r", lambda text, expected=expected: expected in text))
                        session.pump(0.02)  # Pacing is deliberately outside the timed sample.
                    last = json.loads((state / "state.json").read_text())
                    if last["iterations"] <= first["iterations"] or last["done"]:
                        raise RuntimeError("key samples were not collected while the real command streamed")
                    session.capture("key-to-render-during-stream")
                    release.touch()
                    run = await_run(session, data)
                    measurement = {**distribution(values, 50), "stream_before": first, "stream_after": last,
                                   "input": "Enter chooses /follow, alternating visible Follow paused / Follow ON; chooser entry is outside timed sample",
                                   "nominal_workload": "128-byte flushed line every 10 ms plus atomic heartbeat; all child cost included"}
                    name = "key_to_render"
                else:
                    run = await_run(session, data)
                    record = run["record"]
                    spool = data / run["output_name"]
                    size = spool.stat().st_size
                    if not record["truncated"] or size > args.spool_kib * 1024 or record["captured_bytes"] != size:
                        raise RuntimeError("10 MiB workload did not respect its configured retained spool bound")
                    if record["captured_bytes"] + record["discarded_bytes"] != 10 * MIB:
                        raise RuntimeError("10 MiB workload byte accounting is incomplete")
                    shutil.copyfile(spool, output / mode / "retained-output.bin")
                    measurement = {"workload_bytes": 10 * MIB, "write_size_bytes": 4096, "write_count": 2560,
                                   "nominal_write_spacing_seconds": 0.001, "configured_spool_bytes": args.spool_kib * 1024,
                                   "retained_spool_bytes": size, "retained_spool_sha256": digest(spool),
                                   "discarded_bytes": record["discarded_bytes"], "dropped_live_bytes": record["dropped_live_bytes"],
                                   "spool_bound_observed": True, "truncation_visible": "CAPTURE TRUNCATED" in session.text,
                                   "queue_bound_proof": "not inferred from RSS; finite configured-spool exercise only"}
                    name = "output_10_mib"
                measurement.update(duration_seconds=time.monotonic() - workload_started, resources=active.finish(),
                                   argv=argv, execution=run, child=json.loads((state / "state.json").read_text()))
                active = None
                if not measurement["child"]["done"] or Path(measurement["child"]["cwd"]) != project:
                    raise RuntimeError("workload completion/cwd evidence does not match disposable project")
                report["measurements"][name] = measurement
                if not measurement["resources"]["scopes"]["local_commands"]["processes"]:
                    raise RuntimeError("approved workload resource scope was not observed; refusing a core-only measurement")
                save_json(output / mode / "execution.json", run)
                save_json(output / mode / "child.json", measurement["child"])
                session.capture("completed")
                quit_app(session)

            # Exactly 5,000 removed + 5,000 added lines, excluding diff headers.
            repository = root / "diff-project"
            repository.mkdir()
            source = repository / "change.txt"
            source.write_text("".join("before-%05d\n" % number for number in range(5000)), encoding="ascii")
            git = shutil.which("git")
            if not git:
                raise RuntimeError("Git is required for the read-only 10,000-line diff workload")
            for arguments in (("init", "--quiet"), ("add", "--", source.name),
                              ("-c", "user.name=Q02", "-c", "user.email=q02@example.invalid", "commit", "-qm", "disposable baseline")):
                completed = run_owned([git, "-c", "core.autocrlf=false", "-c", "core.hooksPath=", *arguments],
                                      cwd=repository, env=environment, timeout=15)
                if completed.returncode:
                    raise RuntimeError("disposable Git setup failed: " + completed.stderr.decode("utf-8", "replace"))
            source.write_text("".join("after-%05d\n" % number for number in range(5000)), encoding="ascii")
            original = {str(path.relative_to(repository)): digest(path) for path in repository.rglob("*") if path.is_file()}
            data = root / "diff-data"
            active = monitor("diff")
            diff_started = time.monotonic_ns()
            session = launch("diff", data, repository)
            active.session = session
            ready(session)
            direct_input(session, ESC)
            visible(session, lambda text: session.screen.cursor.hidden)
            direct_input(session, "/diff\r")
            visible(session, lambda text: "Repository: baseline captured" in text, timeout=40)
            overview_ms = (time.monotonic_ns() - diff_started) / 1_000_000
            session.capture("repository-overview")
            # Render real diff content, not just the repository-ready status.
            found = False
            for page in range(80):
                if "-before-" in session.display() or "+after-" in session.display():
                    found = True
                    break
                session.send(ESC + "[6~")
            if not found:
                raise RuntimeError("10,000-line source change did not expose actual diff text")
            session.capture("actual-diff-lines")
            scroll_ms = raw_latency(session, ESC + "[F", lambda text: "after-04999" in text)
            session.capture("diff-end")
            duration = (time.monotonic_ns() - diff_started) / 1_000_000_000
            resources = active.finish()
            active = None
            if rows(data)[1] != 0:
                raise RuntimeError("read-only diff created unexpected provider journal entries")
            quit_app(session)
            after = {str(path.relative_to(repository)): digest(path) for path in repository.rglob("*") if path.is_file()}
            if original != after:
                raise RuntimeError("read-only diff changed disposable repository files or Git metadata")
            report["measurements"]["diff_10000_lines"] = {"removed_lines": 5000, "added_lines": 5000,
                "changed_diff_lines_excluding_headers": 10000, "source_bytes": source.stat().st_size,
                "source_sha256": digest(source), "duration_seconds": duration, "ready_overview_ms": overview_ms,
                "end_key_to_visible_last_line_ms": scroll_ms, "resources": resources,
                "timing_scope": "from process/terminal launch, including actual repository inspection, view navigation and evidence capture; not cached-view-only latency",
                "read_only_repository_files_and_metadata": True, "repository_before_sha256": original,
                "repository_after_sha256": after, "internal_queue_bound_proof": "not claimed from one finite workload"}
            for name, value in report["measurements"].items():
                for field in ("target_met", "cpu_target_met", "rss_target_met"):
                    if value.get(field) is False:
                        report["misses"].append(name + ": " + field)
            for resource in monitors:
                if not any(record["scope"] == "core" for record in resource.records.values()):
                    report["misses"].append(resource.directory.name + ": core resource scope unavailable")
                for error in resource.errors:
                    if error.startswith("sampler failed:"):
                        report["misses"].append(resource.directory.name + ": " + error)
            if report["measurements"]["idle"]["terminal_output_bytes"]:
                report["misses"].append("idle: terminal emitted output during the 60-second observation")
            report["status"] = "measured_with_misses" if report["misses"] else "measured_local_targets_met"
    except BaseException as error:
        report["status"] = "failed"
        report["error"] = type(error).__name__ + ": " + str(error)
        (output / "failure.txt").write_text(traceback.format_exc(), encoding="utf-8")
    finally:
        cleanup()
        report["total_duration_seconds"] = time.monotonic() - started
        save_json(output / "results.json", report)
    print(json.dumps({"status": report["status"], "misses": report["misses"], "evidence": str(output / "results.json")}, indent=2))
    return 1 if report["status"] in ("failed", "measured_with_misses") else 0


if __name__ == "__main__":
    raise SystemExit(main())
