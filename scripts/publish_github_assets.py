"""Publish generated ARAM data files to a GitHub release.

Creates (or reuses) a fixed tag release and replaces the named assets, so the
manifest download URL stays stable across game patches:

  https://github.com/{owner}/{repo}/releases/download/aram-data-latest/aram-manifest.json

Requires env GITHUB_TOKEN/GH_TOKEN and GITHUB_REPOSITORY (owner/repo); both are
provided by GitHub Actions. Use --dry-run to print the planned requests.
"""

import argparse
import os
import sys
from pathlib import Path

API_ROOT = "https://api.github.com"
TAG = "aram-data-latest"


def request(method, url, token, dry_run=False, **kwargs):
    if dry_run:
        print("dry-run: %s %s" % (method, url))
        return None
    import requests

    headers = {
        "Authorization": "Bearer %s" % token,
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    response = requests.request(method, url, headers=headers, timeout=60, **kwargs)
    if response.status_code >= 400:
        raise RuntimeError(
            "%s %s -> HTTP %s: %s"
            % (method, url, response.status_code, response.text[:300])
        )
    if response.status_code == 204 or not response.content:
        return None
    return response.json()


def ensure_release(token, repo, dry_run):
    base = "%s/repos/%s" % (API_ROOT, repo)
    release = None
    if dry_run:
        print("dry-run: reuse or create release %s" % TAG)
        return {"id": 0, "tag_name": TAG, "upload_url": "dry-run"}
    import requests

    headers = {
        "Authorization": "Bearer %s" % token,
        "Accept": "application/vnd.github+json",
    }
    response = requests.get(
        "%s/releases/tags/%s" % (base, TAG), headers=headers, timeout=60
    )
    if response.status_code == 200:
        release = response.json()
    elif response.status_code != 404:
        raise RuntimeError(
            "GET release %s -> HTTP %s: %s"
            % (TAG, response.status_code, response.text[:300])
        )
    if release:
        return release
    return request(
        "POST",
        "%s/releases" % base,
        token,
        dry_run=dry_run,
        json={
            "tag_name": TAG,
            "name": "ARAM data (latest)",
            "body": "Automated ARAM balance snapshot "
                    "(wiki.leagueoflegends.com, CC BY-SA 3.0).",
        },
    )


def replace_assets(token, repo, release, files, dry_run):
    base = "%s/repos/%s" % (API_ROOT, repo)
    release_id = release["id"]
    wanted = {path.name for path in files}
    if not dry_run:
        assets = request(
            "GET", "%s/releases/%s/assets" % (base, release_id), token, per_page=100
        ) or []
        for asset in assets:
            if asset.get("name") in wanted:
                request(
                    "DELETE",
                    "%s/releases/assets/%s" % (base, asset["id"]),
                    token,
                )
    else:
        print("dry-run: delete existing assets matching %s" % sorted(wanted))
    import requests

    for path in files:
        if dry_run:
            print("dry-run: upload %s" % path)
            continue
        headers = {
            "Authorization": "Bearer %s" % token,
            "Accept": "application/vnd.github+json",
            "Content-Type": "application/octet-stream",
        }
        with path.open("rb") as stream:
            response = requests.post(
                "%s/releases/%s/assets?name=%s"
                % (base, release_id, requests.utils.quote(path.name)),
                headers=headers,
                data=stream,
                timeout=180,
            )
        if response.status_code >= 400:
            raise RuntimeError(
                "upload %s -> HTTP %s: %s"
                % (path.name, response.status_code, response.text[:300])
            )
        print("uploaded %s" % path.name)


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""))
    parser.add_argument("--files", nargs="+", required=True)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)

    repo = args.repo.strip()
    if "/" not in repo:
        print("GITHUB_REPOSITORY/--repo must be owner/repo", file=sys.stderr)
        return 1
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if not token and not args.dry_run:
        print("GITHUB_TOKEN is not set", file=sys.stderr)
        return 1
    files = [Path(item) for item in args.files]
    for path in files:
        if not path.is_file():
            print("missing file: %s" % path, file=sys.stderr)
            return 1

    release = ensure_release(token, repo, args.dry_run)
    replace_assets(token, repo, release, files, args.dry_run)
    print("Publish to GitHub release %r:" % release.get("tag_name", TAG))
    for path in files:
        print("  https://github.com/%s/releases/download/%s/%s"
              % (repo, TAG, path.name))
    return 0


if __name__ == "__main__":
    sys.exit(main())