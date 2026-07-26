#!/usr/bin/env -S uvx modal serve
"""This is a build and deployment script for topcoat-apps on Modal.

Run this script to preview the deployment. When you're ready, you can deploy the
app to Modal with `uvx modal deploy modal_deploy.py`.
"""

import os
import subprocess
from pathlib import Path

import modal


PROJECT_ROOT = Path(__file__).resolve().parent
REMOTE_APP_DIR = "/app"
SERVER_PORT = 8000

app = modal.App("topcoat-apps")

image = (
    modal.Image.from_registry(
        "rust:1.97-slim-bookworm",
        add_python="3.13",
    )
    .apt_install(
        "build-essential",
        "ca-certificates",
        "clang",
        "curl",
        "fontconfig",
        "fonts-inter",
        "libfftw3-dev",
        "libfontconfig1-dev",
        "libfreetype6-dev",
        "pkg-config",
    )
    .run_commands("cargo install topcoat-cli --version 0.4.0 --locked")
    .workdir(REMOTE_APP_DIR)
    .add_local_dir(
        PROJECT_ROOT,
        remote_path=REMOTE_APP_DIR,
        copy=True,
        ignore=["/target", "**/__pycache__/", "**/.git/", "/modal_deploy.py"],
    )
    .run_commands("topcoat asset bundle --release")
)


@app.function(cpu=1, image=image, scaledown_window=3600, region="us")
@modal.concurrent(max_inputs=128)
@modal.web_server(
    SERVER_PORT,
    startup_timeout=30,
    custom_domains=["topcoat-apps.modal.ekzhang.com"],
)
def topcoat_server() -> None:
    env = os.environ.copy()
    env.update(HOST="0.0.0.0", PORT=str(SERVER_PORT))
    subprocess.Popen(
        ["target/release/topcoat-apps"],
        cwd=REMOTE_APP_DIR,
        env=env,
    )
