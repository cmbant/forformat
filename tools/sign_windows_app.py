"""Sign the forformat Windows release binary using Google Cloud KMS.

The workflow authenticates with google-github-actions/auth before invoking this
script. The same GCP signing secrets used by cmbant/getdist are expected:

- GCP_KMS_KEY
- GCP_CERTIFICATE_CHAIN
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import os
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

KMS_CNG_PROVIDER_URL = (
    "https://github.com/GoogleCloudPlatform/kms-integrations/releases/download/"
    "cng-v1.4/kmscng-1.4-windows-amd64.zip"
)
KMS_CNG_PROVIDER_ARCHIVE = "kmscng-1.4-windows-amd64.zip"
KMS_CNG_PROVIDER_SHA256 = (
    "3c3570e4c7ff6e5ce21874b9cd595227e9e8bbe8183023f8121124c0e80738a3"
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
    tools_dir = Path(tempfile.gettempdir()) / "forformat-signing-tools"
    tools_dir.mkdir(parents=True, exist_ok=True)
    archive = tools_dir / KMS_CNG_PROVIDER_ARCHIVE

    if not archive.exists():
        print(f"Downloading pinned Cloud KMS CNG provider: {KMS_CNG_PROVIDER_ARCHIVE}")
        request = urllib.request.Request(
            KMS_CNG_PROVIDER_URL,
            headers={"User-Agent": "forformat-sign-windows/1.0"},
        )
        with urllib.request.urlopen(request) as response:
            archive.write_bytes(response.read())

    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if digest != KMS_CNG_PROVIDER_SHA256:
        archive.unlink(missing_ok=True)
        raise RuntimeError(
            "Cloud KMS CNG provider checksum mismatch: "
            f"expected {KMS_CNG_PROVIDER_SHA256}, got {digest}"
        )

    print(f"Verified Cloud KMS CNG provider SHA-256: {digest}")
    return archive


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
