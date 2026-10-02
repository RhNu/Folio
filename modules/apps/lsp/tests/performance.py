#!/usr/bin/env python3
"""Repeatable stdio latency probe; use --project/--focus for existing projects.

Generated inputs and real-project reports live under Folio's .folio/lsp-perf.
Existing project files are read only; all probe edits remain in LSP buffers.
"""

import argparse
import hashlib
import json
from pathlib import Path
import queue
import re
import statistics
import subprocess
import threading
import time


class Client:
    def __init__(self, binary, project, log_path, log_filter="info"):
        self.log = log_path.open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            [str(binary), "--log-filter", log_filter, "--log-format", "json", "lsp"],
            cwd=project, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
        )
        self.messages = queue.Queue()
        self.next_id = 0
        self.notifications = []
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        try:
            while True:
                header = self.process.stdout.readline()
                if not header:
                    raise EOFError("language server exited")
                length = int(header.split(b":")[1])
                while self.process.stdout.readline().strip():
                    pass
                data = self.process.stdout.read(length)
                self.messages.put(json.loads(data))
        except Exception as error:
            self.messages.put(error)

    def send(self, method, params=None, request=False):
        message = {"jsonrpc": "2.0", "method": method, "params": params or {}}
        if request:
            self.next_id += 1
            message["id"] = self.next_id
        self.write(message)
        return message.get("id")

    def write(self, message):
        data = json.dumps(message).encode("utf-8")
        self.process.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode("ascii") + data)
        self.process.stdin.flush()

    def wait(self, ids):
        results = {}
        deadline = time.monotonic() + 120
        while set(results) != set(ids):
            message = self.messages.get(timeout=max(0.01, deadline - time.monotonic()))
            if isinstance(message, Exception):
                raise message
            if "id" in message and "method" in message:
                assert message["method"] in ["client/registerCapability", "client/unregisterCapability", "workspace/semanticTokens/refresh", "workspace/codeLens/refresh", "workspace/inlayHint/refresh"], message
                self.notifications.append(message)
                self.write({"jsonrpc": "2.0", "id": message["id"], "result": None})
            elif "id" in message:
                assert message["id"] in ids, message
                results[message["id"]] = message
            else:
                self.notifications.append(message)
        return [results[identifier] for identifier in ids]

    def request(self, method, params=None):
        return self.wait([self.send(method, params, True)])[0]

    def barrier(self):
        # An unknown request provides a queue barrier without triggering analysis.
        result = self.request("folio/benchmarkBarrier")
        assert result["error"]["code"] == -32601, result
        # Loading now happens outside the protocol loop. Transport ordering alone
        # cannot establish project readiness; use the negotiated status generation.
        def status():
            return next((message["params"]["state"] for message in reversed(self.notifications)
                         if message["method"] == "folio/status"), None)
        while status() == "loading":
            message = self.messages.get(timeout=120)
            if isinstance(message, Exception):
                raise message
            assert "method" in message, message
            self.notifications.append(message)
            if "id" in message:
                self.write({"jsonrpc": "2.0", "id": message["id"], "result": None})
        assert status() != "error", self.notifications[-1]

    def finish(self):
        self.request("shutdown")
        self.send("exit")
        self.process.wait(timeout=10)
        self.log.close()
        assert self.process.returncode == 0
        # Old servers can attach the whole project to every log event. Keep useful
        # fields while discarding repeated span dumps from the disposable report.
        path = Path(self.log.name)
        compact = path.with_suffix(".compact")
        with path.open(encoding="utf-8") as source, compact.open("w", encoding="utf-8") as destination:
            for line in source:
                event = json.loads(line)
                event.pop("span", None)
                event.pop("spans", None)
                destination.write(json.dumps(event) + "\n")
        compact.replace(path)

    def cleanup(self):
        """Always reap the process, including assertion failures and timeouts."""
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        for stream in [self.process.stdin, self.process.stdout, self.log]:
            if stream is not None and not stream.closed:
                stream.close()


def script(name, methods, parent=""):
    header = f"ScriptName {name}{parent}\n; Unicode: 雪 🦊\nInt Property Counter Auto\n"
    functions = [
        f"Int Function Work{i}(Int value = 1)\n    Return value + Counter\nEndFunction\n"
        for i in range(methods)
    ]
    return header + "\n".join(functions) + "\nFunction Run()\n    Int current = Work0(1)\nEndFunction\n"


def make_project(root, count, methods, dependencies):
    sources = root / "Source/Scripts"
    sources.mkdir(parents=True, exist_ok=True)
    manifest = '''[package]
name = "lsp-benchmark"
version = "0.1.0"
[languages.papyrus]
dialect = "skyrim"
extensions = ["psc"]
[build]
target = "skyrim-se"
profile = "dev"
emit = ["pex"]
'''
    if dependencies:
        manifest += '''[[dependencies]]
name = "api"
kind = "psc"
path = "api"
'''
        api = root / "api"
        api.mkdir(exist_ok=True)
        for i in range(120):
            (api / f"Api{i:03}.psc").write_text(
                f"ScriptName Api{i:03}\n" + "".join(
                    f"Int Function Value{j}(Int count = 1) Native\n" for j in range(20)
                ), encoding="utf-8"
            )
    (root / "folio.toml").write_text(manifest, encoding="utf-8")
    for i in range(count):
        (sources / f"Bench{i:03}.psc").write_text(script(f"Bench{i:03}", methods), encoding="utf-8")
    return sources / "Bench000.psc"


def position(text, marker):
    prefix = text[:text.rindex(marker)]
    return {"line": prefix.count("\n"), "character": len(prefix.rsplit("\n", 1)[-1])}


def bundle(client, uri, text):
    client.barrier()
    common = {"textDocument": {"uri": uri}}
    requests = [
        ("textDocument/documentSymbol", common),
        ("textDocument/semanticTokens/full", common),
        ("textDocument/hover", {**common, "position": position(text, "Work0(1)")}),
        ("textDocument/definition", {**common, "position": position(text, "Work0(1)")}),
        ("textDocument/signatureHelp", {**common, "position": position(text, "1)\nEndFunction")}),
    ]
    responses = client.wait([client.send(method, params, True) for method, params in requests])
    for response in responses:
        assert "error" not in response, response
    result = [response["result"] for response in responses]
    assert result[0][0]["name"] == "Bench000", result[0]
    assert result[1]["data"] and result[2] and result[3] and result[4], result
    assert "Int Function Work0" in result[2]["contents"]["value"], result[2]
    assert result[3]["range"]["start"]["line"] == 3, result[3]
    return result


def measured(action):
    started = time.perf_counter()
    result = action()
    return (time.perf_counter() - started) * 1000, result


def run_generated(client, root, focus, encoding):
    started = time.perf_counter()
    initialized = client.request("initialize", {
        "rootUri": root.as_uri(),
        "initializationOptions": {"folio": {"status": True}},
        "capabilities": {"general": {"positionEncodings": [encoding]}},
    })
    assert initialized["result"]["capabilities"]["positionEncoding"] == encoding
    metrics = {"initialize_reply_ms": (time.perf_counter() - started) * 1000}
    client.send("initialized")
    metrics["initial_load_ms"], _ = measured(client.barrier)
    text = focus.read_text(encoding="utf-8")
    uri = focus.as_uri()
    document = {"uri": uri, "languageId": "papyrus", "version": 1, "text": text}

    def opened():
        client.send("textDocument/didOpen", {"textDocument": document})
        return bundle(client, uri, text)

    metrics["first_open_ms"], first = measured(opened)
    metrics["cold_symbols_ms"] = (time.perf_counter() - started) * 1000
    original_diagnostics = next(
        message["params"]["diagnostics"] for message in reversed(client.notifications)
        if message["method"] == "textDocument/publishDiagnostics" and message["params"]["uri"] == uri
    )
    assert not original_diagnostics, original_diagnostics
    metrics["warm_requests_ms"], warm = measured(lambda: bundle(client, uri, text))
    assert first == warm

    def closed():
        client.send("textDocument/didClose", {"textDocument": {"uri": uri}})
        client.barrier()

    metrics["close_ms"], _ = measured(closed)
    metrics["reopen_ms"], reopened = measured(opened)
    assert first == reopened
    changed = text.replace("value + Counter", "value + missing", 1)

    def change():
        client.send("textDocument/didChange", {
            "textDocument": {"uri": uri, "version": 2},
            "contentChanges": [{"text": changed}],
        })
        return bundle(client, uri, changed)

    metrics["change_ms"], _ = measured(change)
    changed_diagnostics = next(
        message["params"] for message in reversed(client.notifications)
        if message["method"] == "textDocument/publishDiagnostics" and message["params"]["uri"] == uri
    )
    assert changed_diagnostics["version"] == 2 and any(
        "missing" in diagnostic["message"] for diagnostic in changed_diagnostics["diagnostics"]
    ), changed_diagnostics
    metrics["discard_close_ms"], _ = measured(closed)
    metrics["restored_open_ms"], restored = measured(opened)
    assert first == restored
    assert next(
        message["params"]["diagnostics"] for message in reversed(client.notifications)
        if message["method"] == "textDocument/publishDiagnostics" and message["params"]["uri"] == uri
    ) == original_diagnostics
    client.finish()
    payload = json.dumps(first, sort_keys=True, ensure_ascii=False).encode("utf-8")
    return metrics, hashlib.sha256(payload).hexdigest(), first


def run_real(client, root, focus, encoding):
    """Exercise a supplied project exclusively through unsaved LSP buffers."""
    started = time.perf_counter()
    initialized = client.request("initialize", {
        "rootUri": root.as_uri(),
        "initializationOptions": {"folio": {"status": True}},
        "capabilities": {"general": {"positionEncodings": [encoding]}},
    })
    assert "error" not in initialized, initialized
    assert initialized["result"]["capabilities"]["positionEncoding"] == encoding
    metrics = {"initialize_reply_ms": (time.perf_counter() - started) * 1000}
    client.send("initialized")
    metrics["initial_load_ms"], _ = measured(client.barrier)
    original = focus.read_text(encoding="utf-8-sig")
    header = re.search(r"(?im)^\s*scriptname\s+([A-Za-z_][A-Za-z0-9_]*)", original)
    assert header, "focus must contain a ScriptName header"
    script_name = header.group(1)
    # The extra newline provides a repeatable empty-prefix completion position.
    text = original + "\n"
    active_text = text
    uri = focus.as_uri()
    common = {"textDocument": {"uri": uri}}
    version = 1

    def at(content, index):
        before = content[:index]
        column = before.rsplit("\n", 1)[-1]
        units = len(column.encode("utf-16-le")) // 2 if encoding == "utf-16" else len(column.encode("utf-8"))
        return {"line": before.count("\n"), "character": units}

    def request(method, params=None):
        response = client.request(method, params or common)
        assert "error" not in response, response
        return response["result"]

    def change(content):
        nonlocal version, active_text
        version += 1
        active_text = content
        client.send("textDocument/didChange", {
            "textDocument": {"uri": uri, "version": version},
            "contentChanges": [{"text": content}],
        })
        client.barrier()

    def symbols():
        result = request("textDocument/documentSymbol")
        assert result and result[0]["name"].lower() == script_name.lower(), result
        assert result[0]["range"]["end"] == at(active_text, len(active_text)), "symbols use an obsolete buffer range"
        return result

    def declarations(result):
        # The script's containing range grows with trailing comments; declaration
        # identities, selection ranges, and every member span must remain exact.
        normalized = json.loads(json.dumps(result))
        normalized[0]["range"].pop("end")
        return normalized

    def completion(content):
        result = request("textDocument/completion", {
            **common, "position": at(content, len(content)),
        })
        items = result["items"] if isinstance(result, dict) else result
        assert items, "completion returned no candidates"
        return items

    def open_document():
        client.send("textDocument/didOpen", {"textDocument": {
            "uri": uri, "languageId": "papyrus", "version": version, "text": text,
        }})
        client.barrier()
        return symbols()

    metrics["first_open_ms"], first = measured(open_document)
    metrics["warm_symbols_ms"], warm = measured(symbols)
    assert first == warm, "warm symbols differ from the opened snapshot"
    metrics["header_hover_ms"], hover = measured(lambda: request("textDocument/hover", {
        **common, "position": at(text, header.start(1)),
    }))
    assert hover and script_name.lower() in json.dumps(hover, ensure_ascii=False).lower(), hover
    metrics["empty_completion_ms"], candidates = measured(lambda: completion(text))
    selected = next((item for item in candidates if item.get("data") and
                     re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", item["label"])), None)
    assert selected, "completion has no resolvable semantic candidate"
    metrics["completion_resolve_ms"], resolved = measured(lambda: request("completionItem/resolve", selected))
    assert resolved["label"] == selected["label"], resolved
    prefix = selected["label"][:min(3, len(selected["label"]))]

    def narrow():
        narrowed_text = text + prefix
        change(narrowed_text)
        return completion(narrowed_text)

    metrics["narrow_completion_ms"], narrowed = measured(narrow)
    assert all(item["label"].lower().startswith(prefix.lower()) for item in narrowed), narrowed
    assert selected["label"] in [item["label"] for item in narrowed], narrowed
    assert len(narrowed) <= len(candidates)

    def edited():
        change(text + "; Folio performance probe: unsaved comment\n")
        return symbols()

    metrics["comment_edit_ms"], edited_symbols = measured(edited)
    assert declarations(edited_symbols) == declarations(first), "comment edit changed declaration symbols"

    def saved():
        client.send("textDocument/didSave", common)
        client.barrier()
        return symbols()

    metrics["save_refresh_ms"], saved_symbols = measured(saved)
    assert declarations(saved_symbols) == declarations(first), "save refresh published stale or altered symbols"

    def restored():
        change(text)
        result = symbols()
        client.send("textDocument/didClose", common)
        client.barrier()
        return result

    metrics["restore_close_ms"], restored_symbols = measured(restored)
    assert restored_symbols == first, "restoring the original buffer changed symbols"
    client.finish()
    # Response data contains session generations; retain stable public output only.
    payload = {"script": script_name, "symbols": first, "hover": hover,
               "candidate_count": len(candidates), "narrow_candidate_count": len(narrowed),
               "resolved_label": resolved["label"],
               "completion_labels": sorted(item["label"] for item in candidates)}
    digest = hashlib.sha256(json.dumps(payload, sort_keys=True, ensure_ascii=False).encode("utf-8")).hexdigest()
    return metrics, digest, payload


def run(binary, root, focus, log_path, encoding, log_filter, real_project=False):
    original_bytes = focus.read_bytes() if real_project else None
    client = Client(binary, root, log_path, log_filter)
    try:
        return (run_real if real_project else run_generated)(client, root, focus, encoding)
    finally:
        client.cleanup()
        if real_project:
            assert focus.read_bytes() == original_bytes, "focus file changed on disk during the probe"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compare", type=Path)
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--encoding", choices=["utf-8", "utf-16"], default="utf-16")
    parser.add_argument("--log-filter", default="info")
    parser.add_argument("--project", type=Path, help="existing project directory; never modified by the probe")
    parser.add_argument("--focus", type=Path, help="PSC path relative to --project")
    args = parser.parse_args()
    if args.samples < 1:
        parser.error("--samples must be positive")
    if bool(args.project) != bool(args.focus):
        parser.error("--project and --focus must be supplied together")
    repository = Path(__file__).resolve().parents[4]
    work = repository / ".folio/lsp-perf"
    work.mkdir(parents=True, exist_ok=True)
    reports = {}
    cases = [
        ("small", 1, 20, False),
        ("project", 64, 24, True),
        ("long", 1, 800, False),
    ]
    if args.project:
        root = args.project.resolve()
        if work.is_relative_to(root):
            parser.error("the existing project must be outside Folio's report directory ancestry")
        focus = (root / args.focus).resolve()
        if args.focus.is_absolute() or not focus.is_relative_to(root) or not focus.is_file():
            parser.error("--focus must identify an existing file inside --project")
        if not (root / "folio.toml").is_file():
            parser.error("--project must contain folio.toml")
        cases = [("real", None, None, False)]
        # Keep artifacts in Folio's disposable report directory, never the supplied project.
        args.output = work / args.output.name
    for name, count, methods, dependencies in cases:
        if not args.project:
            root = work / name
            focus = make_project(root, count, methods, dependencies)
        runs = []
        hashes = []
        for sample in range(args.samples):
            log = work / f"{args.output.stem}-{name}-{sample}.jsonl"
            metrics, digest, payload = run(args.binary.resolve(), root, focus, log, args.encoding, args.log_filter, bool(args.project))
            runs.append(metrics)
            hashes.append(digest)
            (work / f"{args.output.stem}-{name}-symbols.json").write_text(
                json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8"
            )
        assert len(set(hashes)) == 1
        reports[name] = {
            "median_ms": {key: round(statistics.median(run[key] for run in runs), 3) for key in runs[0]},
            "runs": runs, "symbol_hash": hashes[0],
            "sources": count, "functions_per_source": methods,
            "encoding": args.encoding, "log_filter": args.log_filter,
            "log_format": "json",
        }
        if args.project:
            reports[name].update({"project": str(root), "focus": str(args.focus),
                                  "candidate_count": payload["candidate_count"],
                                  "narrow_candidate_count": payload["narrow_candidate_count"]})
        print(name, json.dumps(reports[name]["median_ms"]), flush=True)
    if args.compare:
        baseline = json.loads(args.compare.read_text(encoding="utf-8"))
        for name in reports:
            assert reports[name]["symbol_hash"] == baseline[name]["symbol_hash"], name
        print("Symbols, hover and completion candidates match the baseline." if args.project else
              "Symbols, token spans, hover, definition and signature match the baseline.")
    args.output.write_text(json.dumps(reports, indent=2) + "\n", encoding="utf-8")
    print(f"Report: {args.output}", flush=True)


if __name__ == "__main__":
    main()
