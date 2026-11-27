"""Publish generated ARAM data files to a Gitee release.

Creates (or reuses) a fixed tag release and replaces the named attachments, so
the manifest download URL stays stable across game patches:

  https://gitee.com/{owner}/{repo}/releases/download/aram-data-latest/aram-manifest.json

Requires env GITEE_TOKEN (personal access token with repo write access).
"""

import argparse
import os
import sys
from pathlib import Path

API_ROOT = "https://gitee.com/api/v5"


def request(method, url, **kwargs):
    import requests

    response = requests.request(method, url, timeout=30, **kwargs)
    if response.status_code >= 400:
        raise RuntimeError(
            "%s %s -> HTTP %s: %s"
            % (method, url, response.status_code, response.text[:300])
        )
    if response.status_code == 204 or not response.content:
        return None
    return response.json()


def ensure_release(session_token, owner, repo, tag, name, body, target):
    base = "%s/repos/%s/%s" % (API_ROOT, owner, repo)
    release = None
    try:
        release = request(
            "GET", "%s/releases/tags/%s" % (base, tag),
            params={"access_token": session_token},
        )
    except RuntimeError as error:
        if "HTTP 404" not in str(error):
            raise
    if release:
        return release
    return request(
        "POST", "%s/releases" % base,
        data={
            "access_token": session_token,
            "tag_name": tag,
            "name": name,
            "body": body,
            "target_commitish": target,
        },
    )


def replace_attachments(token, owner, repo, release, files):
    base = "%s/repos/%s/%s" % (API_ROOT, owner, repo)
    release_id = release["id"]
    wanted = {path.name for path in files}
    existing = request(
        "GET", "%s/releases/%s/attach_files" % (base, release_id),
        params={"access_token": token, "per_page": 100},
    ) or []
    for attachment in existing:
        if attachment.get("name") in wanted:
            request(
                "DELETE",
                "%s/releases/%s/attach_files/%s" % (base, release_id, attachment["id"]),
                params={"access_token": token},
            )
    import requests

    urls = []
    for path in files:
        with path.open("rb") as stream:
            response = requests.post(
                "%s/releases/%s/attach_files" % (base, release_id),
                data={"access_token": token},
                files={"file": (path.name, stream)},
                timeout=120,
            )
        if response.status_code >= 400:
            raise RuntimeError(
                "upload %s -> HTTP %s: %s"
                % (path.name, response.status_code, response.text[:300])
            )
        urls.append(
            "https://gitee.com/%s/%s/releases/download/%s/%s"
            % (owner, repo, release["tag_name"], path.name)
        )
    return urls


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--owner", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--tag", default="aram-data-latest")
    parser.add_argument("--release-name", default="ARAM data (latest)")
    parser.add_argument(
        "--release-body",
        default="Automated ARAM balance snapshot (wiki.leagueoflegends.com, CC BY-SA 3.0).",
    )
    parser.add_argument("--target", default="main")
    parser.add_argument("--files", nargs="+", required=True)
    args = parser.parse_args(argv)

    token = os.environ.get("GITEE_TOKEN", "").strip()
    if not token:
        print("GITEE_TOKEN is not set; skipping Gitee publish.", file=sys.stderr)
        return 1

    files = [Path(item) for item in args.files]
    for path in files:
        if not path.is_file():
            print("missing file: %s" % path, file=sys.stderr)
            return 1

    release = ensure_release(
        token, args.owner, args.repo, args.tag, args.release_name, args.release_body, args.target
    )
    urls = replace_attachments(token, args.owner, args.repo, release, files)
    print("Published to Gitee release %r:" % release.get("tag_name"))
    for url in urls:
        print("  " + url)
    manifest = next((url for url in urls if url.endswith("aram-manifest.json")), None)
    if manifest:
        print("Set SERAPHINE_ARAM_DATA_URL or Settings > AramDataFeedUrl to:")
        print("  " + manifest)
    return 0


if __name__ == "__main__":
    sys.exit(main())