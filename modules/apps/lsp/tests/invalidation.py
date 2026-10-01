#!/usr/bin/env python3
"""Exercise a real stdio server's buffer, dependency and filesystem invalidation."""

import argparse
import json
from pathlib import Path
import shutil

from performance import Client


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[4] / ".folio/lsp-invalidation"
    if root.exists():
        shutil.rmtree(root)
    sources = root / "Source/Scripts"
    sources.mkdir(parents=True)
    api = root.parent / "lsp-invalidation-api"
    if api.exists():
        shutil.rmtree(api)
    api.mkdir()
    parent = api / "Base.psc"
    parent.write_text("ScriptName Base\nInt Function Value(Int amount) Native\n")
    manifest = '''schema = 3
[package]
name = "invalidation"
version = "0.1.0"
[languages.papyrus]
dialect = "skyrim"
extensions = ["psc"]
[build]
target = "skyrim-se"
profile = "dev"
emit = ["pex"]
[[dependencies]]
name = "api"
kind = "psc"
path = "../lsp-invalidation-api"
'''
    manifest_path = root / "folio.toml"
    manifest_path.write_text(manifest)
    focus = sources / "Child.psc"
    original = "ScriptName Child Extends Base\nFunction Run()\n    Value(1)\nEndFunction\n"
    focus.write_text(original)
    uri = focus.as_uri()
    client = Client(args.binary.resolve(), root, root / "server.jsonl", "debug")
    client.request("initialize", {"rootUri": root.as_uri(), "capabilities": {
        "workspace": {"didChangeWatchedFiles": {"dynamicRegistration": True, "relativePatternSupport": True}},
    }})
    client.send("initialized")
    client.barrier()
    registrations = [message for message in client.notifications if message["method"] == "client/registerCapability"]
    assert registrations, "server did not register dependency watches"
    watchers = registrations[-1]["params"]["registrations"][0]["registerOptions"]["watchers"]
    assert any(watch["globPattern"]["baseUri"] == api.as_uri() for watch in watchers), watchers

    def notify(method, params):
        client.send(method, params)
        client.barrier()

    def watch(path, kind=2):
        notify("workspace/didChangeWatchedFiles", {"changes": [{"uri": path.as_uri(), "type": kind}]})

    def open_file(path, text, version=1):
        notify("textDocument/didOpen", {"textDocument": {
            "uri": path.as_uri(), "languageId": "papyrus", "version": version, "text": text,
        }})

    def close_file(path):
        notify("textDocument/didClose", {"textDocument": {"uri": path.as_uri()}})

    def change(text, version):
        notify("textDocument/didChange", {
            "textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": text}],
        })

    def outline(path=focus):
        result = client.request("textDocument/documentSymbol", {"textDocument": {"uri": path.as_uri()}})
        assert "error" not in result, result
        return result["result"]

    def hover_value():
        result = client.request("textDocument/hover", {
            "textDocument": {"uri": uri}, "position": {"line": 2, "character": 6},
        })
        assert "error" not in result, result
        return result["result"]

    def diagnostics():
        return next(message["params"] for message in reversed(client.notifications)
                    if message["method"] == "textDocument/publishDiagnostics"
                    and message["params"]["uri"] == uri)

    open_file(focus, original)
    assert "Int Function Value" in hover_value()["contents"]["value"]
    assert not diagnostics()["diagnostics"]
    parent.write_text(parent.read_text().replace("Int Function", "Float Function"))
    watch(parent)
    assert "Float Function Value" in hover_value()["contents"]["value"]

    # Dependency order selects a complete provider, including its current origin.
    override = root / "override"
    override.mkdir()
    (override / "Base.psc").write_text("ScriptName Base\nBool Function Value(Int amount) Native\n")
    manifest_path.write_text(manifest + '\n[[dependencies]]\nname = "override"\nkind = "psc"\npath = "override"\n')
    watch(manifest_path)
    watchers = next(message for message in reversed(client.notifications) if message["method"] == "client/registerCapability")["params"]["registrations"][0]["registerOptions"]["watchers"]
    assert any(watch["globPattern"]["baseUri"] == override.as_uri() for watch in watchers), watchers
    item = hover_value()["contents"]["value"]
    assert "Bool Function Value" in item and "override" in item, item
    manifest_path.write_text(manifest)
    watch(manifest_path)
    assert "Float Function Value" in hover_value()["contents"]["value"]

    # A saved or watched file does not supersede an open buffer.
    disk = original.replace("Run", "DiskRun")
    focus.write_text(disk)
    watch(focus)
    assert outline()[0]["children"][0]["name"] == "Run"
    notify("textDocument/didSave", {"textDocument": {"uri": uri}})
    assert outline()[0]["children"][0]["name"] == "Run"
    close_file(focus)
    assert outline()[0]["children"][0]["name"] == "DiskRun"
    open_file(focus, disk)
    focus.write_text(original.replace("Run", "UnwatchedRun"))
    close_file(focus)
    assert outline()[0]["children"][0]["name"] == "UnwatchedRun"
    focus.write_text(original)
    watch(focus)
    open_file(focus, original)

    # Editing, stale versions and manifest call policy operate on one coherent view.
    missing = original.replace("Value(1)", "Value()")
    change(missing, 2)
    assert diagnostics()["version"] == 2 and diagnostics()["diagnostics"]
    notify("textDocument/didChange", {
        "textDocument": {"uri": uri, "version": 1}, "contentChanges": [{"text": original}],
    })
    assert diagnostics()["version"] == 2
    manifest_path.write_text(manifest.replace('extensions = ["psc"]', 'extensions = ["psc"]\nfill-missing-arguments = true'))
    watch(manifest_path)
    assert all(item["severity"] != 1 for item in diagnostics()["diagnostics"])
    manifest_path.write_text(manifest)
    watch(manifest_path)
    assert any(item["severity"] == 1 for item in diagnostics()["diagnostics"])
    change(original, 3)
    assert not diagnostics()["diagnostics"]

    # New overlays enter the root graph and disappear when discarded.
    new = sources / "New.psc"
    open_file(new, "ScriptName New\nFunction Fresh() Native\n")
    assert outline(new)[0]["name"] == "New"
    close_file(new)
    assert outline(new) is None
    new.write_text("ScriptName New\nFunction Saved() Native\n")
    watch(new, 1)
    assert outline(new)[0]["children"][0]["name"] == "Saved"
    new.unlink()
    watch(new, 3)
    assert outline(new) is None

    # Deleting an open source retains its overlay only until close.
    focus.unlink()
    watch(focus, 3)
    assert outline()[0]["name"] == "Child"
    close_file(focus)
    assert outline() is None
    focus.write_text(original)
    watch(focus, 1)
    assert outline()[0]["name"] == "Child"

    # Failed project loads clear stale results; fixing the input recovers the session.
    manifest_path.write_text("schema = [broken\n")
    watch(manifest_path)
    assert outline() is None
    manifest_path.write_text(manifest)
    watch(manifest_path)
    assert outline()[0]["name"] == "Child"
    assert "Float Function Value" in hover_value()["contents"]["value"]
    client.finish()
    print(json.dumps({"status": "passed", "checks": [
        "external dependency watch registration", "PSC dependency edits", "provider order and origin", "save and disk overlays",
        "unwatched close restore", "buffer revisions", "manifest call policy",
        "unsaved source graph", "file create/delete", "deleted open source",
        "project load failure and recovery",
    ]}, ensure_ascii=False))


if __name__ == "__main__":
    main()
