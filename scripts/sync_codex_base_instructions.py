#!/usr/bin/env python3

import hashlib
import json
import os
import urllib.request
from pathlib import Path
from typing import Any


MODELS_PATH = "codex-rs/models-manager/models.json"
COMMITS_URL = (
    "https://api.github.com/repos/openai/codex/commits"
    f"?sha=main&path={MODELS_PATH}&per_page=1"
)
RAW_URL_TEMPLATE = (
    "https://raw.githubusercontent.com/openai/codex/{commit}/"
    f"{MODELS_PATH}"
)
TARGET = Path("crates/protocol/src/prompts/base_instructions/default.md")
EXCLUDED_SLUGS = {"codex-auto-review"}


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def emit_output(name: str, value: str) -> None:
    output_path = os.environ.get("GITHUB_OUTPUT")
    if output_path:
        with Path(output_path).open("a", encoding="utf-8") as handle:
            handle.write(f"{name}={value}\n")
    print(f"{name}={value}")


def github_request(url: str) -> urllib.request.Request:
    return urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "codex-shim-sync-base-instructions",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )


def latest_models_commit() -> tuple[str, str]:
    with urllib.request.urlopen(github_request(COMMITS_URL), timeout=30) as response:
        commits = json.loads(response.read().decode("utf-8"))
    if not isinstance(commits, list) or len(commits) != 1:
        raise SystemExit("expected one latest openai/codex commit for models.json")

    commit = commits[0]
    if not isinstance(commit, dict):
        raise SystemExit("latest openai/codex commit payload must be an object")

    sha = commit.get("sha")
    url = commit.get("html_url")
    if not isinstance(sha, str) or not sha:
        raise SystemExit("latest openai/codex commit must have a sha")
    if not isinstance(url, str) or not url:
        raise SystemExit("latest openai/codex commit must have an html_url")
    return sha, url


def read_models_json(commit: str) -> str:
    request = urllib.request.Request(
        RAW_URL_TEMPLATE.format(commit=commit),
        headers={"User-Agent": "codex-shim-sync-base-instructions"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read().decode("utf-8")


def select_model(data: dict[str, Any]) -> dict[str, Any]:
    models = data.get("models")
    if not isinstance(models, list):
        raise SystemExit("models.json must contain a models array")

    candidates: list[dict[str, Any]] = []
    for model in models:
        if not isinstance(model, dict):
            continue
        if model.get("visibility") != "list":
            continue
        if model.get("slug") in EXCLUDED_SLUGS:
            continue
        if not isinstance(model.get("priority"), int):
            continue
        if not isinstance(model.get("base_instructions"), str):
            continue
        if not model["base_instructions"].strip():
            continue
        candidates.append(model)

    if not candidates:
        raise SystemExit("no list-visible model with priority and base_instructions found")

    min_priority = min(model["priority"] for model in candidates)
    selected = [model for model in candidates if model["priority"] == min_priority]
    if len(selected) != 1:
        slugs = ", ".join(sorted(str(model.get("slug")) for model in selected))
        raise SystemExit(f"multiple models share priority {min_priority}: {slugs}")

    return selected[0]


def main() -> int:
    source_commit, source_commit_url = latest_models_commit()
    raw = read_models_json(source_commit)
    data = json.loads(raw)
    if not isinstance(data, dict):
        raise SystemExit("models.json root must be an object")

    model = select_model(data)
    slug = model.get("slug")
    if not isinstance(slug, str) or not slug:
        raise SystemExit("selected model must have a slug")

    base_instructions = model["base_instructions"].rstrip() + "\n"
    previous = TARGET.read_text(encoding="utf-8")

    changed = previous != base_instructions
    if changed:
        TARGET.write_text(base_instructions, encoding="utf-8")

    emit_output("changed", "true" if changed else "false")
    emit_output("model", slug)
    emit_output("source_commit", source_commit)
    emit_output("source_commit_short", source_commit[:12])
    emit_output("source_commit_url", source_commit_url)
    emit_output("source_sha256", sha256_text(raw))
    emit_output("prompt_sha256", sha256_text(base_instructions))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
