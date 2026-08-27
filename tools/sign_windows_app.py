"""Sign the forformat Windows release binary using Google Cloud KMS.

The workflow authenticates with google-github-actions/auth before invoking this
script. The same GCP signing secrets used by cmbant/getdist are expected:

- GCP_KMS_KEY
- GCP_CERTIFICATE_CHAIN
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

KMS_RELEASES_API = (
    "https://api.github.com/repos/GoogleCloudPlatform/kms-integrations/"
    "releases?per_page=20"
)
KMS_ARCHIVE_RE = re.compile(
    r"^kmscng-[0-9][0-9A-Za-z.\-]*-windows-amd64\.zip$"
)


def find_signtool() -> str:
    try:
        subprocess.check_call(
            ["where", "signtool"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return "signtool"
    except subprocess.CalledProcessError:
        pass

    kits = Path(r"C:\Program Files (x86)\Windows Kits\10\bin")
    if kits.is_dir():
        versions = sorted(
            (
                path
                for path in kits.iterdir()
                if path.is_dir() and path.name.startswith("10.")
            ),
            key=lambda path: tuple(
                int(part) for part in path.name.split(".") if part.isdigit()
            ),
            reverse=True,
        )
        for version in versions:
            candidate = version / "x64" / "signtool.exe"
            if candidate.is_file():
                return str(candidate)

    raise RuntimeError("Could not find signtool.exe")


def download_cng_provider() -> Path:
    request = urllib.request.Request(
        KMS_RELEASES_API,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "forformat-sign-windows/1.0",
        },
    )
    with urllib.request.urlopen(request) as response:
        releases = json.loads(response.read().decode("utf-8"))

    candidates = []
    for release in releases:
        tag = release.get("tag_name", "")
        if (
            not tag.startswith("cng-v")
            or release.get("draft")
            or release.get("prerelease")
        ):
            continue
        version = tuple(
            int(part)
            for part in tag.removeprefix("cng-v").split(".")
            if part.isdigit()
        )
        candidates.append((version, release.get("published_at") or "", release))

    for _, _, release in sorted(
        candidates, key=lambda item: (item[0], item[1]), reverse=True
    ):
        for asset in release.get("assets", []):
            name = asset.get("name", "")
            if KMS_ARCHIVE_RE.fullmatch(name):
                tools_dir = Path(tempfile.gettempdir()) / "forformat-signing-tools"
                tools_dir.mkdir(parents=True, exist_ok=True)
                archive = tools_dir / name
                if not archive.exists():
                    print(f"Downloading {release['tag_name']} {name}")
                    asset_request = urllib.request.Request(
                        asset["browser_download_url"],
                        headers={"User-Agent": "forformat-sign-windows/1.0"},
                    )
                    with urllib.request.urlopen(asset_request) as response:
                        archive.write_bytes(response.read())
                return archive

    raise RuntimeError("Could not resolve the Google Cloud KMS CNG provider")


def install_cng_provider() -> None:
    archive = download_cng_provider()
    extract_dir = archive.with_suffix("")
    if not extract_dir.is_dir():
        extract_dir.mkdir(parents=True)
        with zipfile.ZipFile(archive) as package:
            package.extractall(extract_dir)

    installers = sorted(extract_dir.rglob("*.msi"))
    if not installers:
        raise RuntimeError(f"No MSI installer found under {extract_dir}")

    subprocess.check_call(
        ["msiexec", "/i", str(installers[0]), "/quiet", "/norestart"]
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--file", required=True, help="Executable to sign")
    parser.add_argument(
        "--install-cng",
        action="store_true",
        help="Install the Google Cloud KMS CNG provider first",
    )
    args = parser.parse_args()

    if sys.platform != "win32":
        raise RuntimeError("Windows signing must run on Windows")

    executable = Path(args.file)
    if not executable.is_file():
        raise RuntimeError(f"Release executable not found: {executable}")

    kms_key = os.environ.get("GCP_KMS_KEY")
    certificate_b64 = os.environ.get("GCP_CERTIFICATE_CHAIN")
    if not kms_key or not certificate_b64:
        raise RuntimeError(
            "GCP_KMS_KEY and GCP_CERTIFICATE_CHAIN must both be set"
        )

    if args.install_cng:
        install_cng_provider()

    signtool = find_signtool()
    with tempfile.TemporaryDirectory() as temp_dir:
        certificate = Path(temp_dir) / "certificate-chain.crt"
        certificate.write_bytes(base64.b64decode(certificate_b64))

        subprocess.check_call(
            [
                signtool,
                "sign",
                "/v",
                "/fd",
                "sha256",
                "/td",
                "sha256",
                "/tr",
                "http://timestamp.digicert.com",
                "/f",
                str(certificate),
                "/csp",
                "Google Cloud KMS Provider",
                "/kc",
                kms_key,
                "/d",
                "forformat",
                str(executable),
            ]
        )
        subprocess.check_call(
            [signtool, "verify", "/pa", "/v", str(executable)]
        )


if __name__ == "__main__":
    main()
