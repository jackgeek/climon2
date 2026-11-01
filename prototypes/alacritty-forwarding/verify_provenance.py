"""Throwaway #56 locked-source/licence inventory, not a vulnerability audit."""

import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile


ROOT = Path(__file__).parent
REVISION = "bbb2cbf9470a6707b1205fbddbdb25109b14e2fd"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def verify_archive(root, archive_path, expected_checksum):
    if sha256(archive_path.read_bytes()) != expected_checksum:
        raise ValueError(f"Archive checksum differs: {root.name}")
    checked = 0
    with tarfile.open(archive_path, "r:gz") as archive:
        for member in archive:
            path = Path(member.name)
            if path.is_absolute() or path.parts[0] != root.name or ".." in path.parts:
                raise ValueError(f"Invalid archive path: {root.name}")
            if member.isdir():
                continue
            if not member.isfile():
                raise ValueError(f"Unsupported archive entry: {root.name}")
            local = root / Path(*path.parts[1:])
            with archive.extractfile(member) as source:
                if sha256(local.read_bytes()) != sha256(source.read()):
                    raise ValueError(f"Source differs: {root.name}/{local.relative_to(root)}")
            checked += 1
    if not checked:
        raise ValueError(f"Archive contains no source files: {root.name}")
    return checked


def license_files(root):
    names = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE")
    return [
        {"path": path.name, "sha256": sha256(path.read_bytes())}
        for path in sorted(root.iterdir())
        if path.is_file() and path.name.upper().startswith(names)
    ]


def inventory():
    upstream = ROOT / ".cache/alacritty"
    git = ["git", "-C", str(upstream)]
    revision = subprocess.check_output(git + ["rev-parse", "HEAD"], text=True).strip()
    if revision != REVISION or subprocess.check_output(git + ["status", "--porcelain"], text=True):
        raise ValueError("Unexpected or modified Alacritty source")
    licenses = license_files(upstream)
    if not any(item["path"] == "LICENSE-APACHE" for item in licenses):
        raise ValueError("Upstream Apache licence is missing")
    lock = (ROOT / "probe/Cargo.lock").read_text()
    checksums = {}
    for section in lock.split("[[package]]")[1:]:
        fields = dict(re.findall(r'^(name|version|checksum) = "([^"]+)"$', section, re.MULTILINE))
        checksums[(fields["name"], fields["version"])] = fields.get("checksum")
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--offline", "--format-version", "1",
        "--manifest-path", str(ROOT / "probe/Cargo.toml"),
    ], text=True))
    packages = []
    for package in metadata["packages"]:
        if package["source"] is None:
            continue
        root = Path(package["manifest_path"]).parent
        archive = root.parents[2] / "cache" / root.parent.name / (root.name + ".crate")
        checksum = checksums[(package["name"], package["version"])]
        checked = verify_archive(root, archive, checksum)
        licenses_for_package = license_files(root)
        if not package["license"] or not licenses_for_package:
            raise ValueError(f"Missing licence evidence: {root.name}")
        packages.append({
            "name": package["name"], "version": package["version"],
            "source": package["source"], "lock_checksum": checksum,
            "archive_sha256_verified": True, "source_file_sha256_checks": checked,
            "license": package["license"], "license_files": licenses_for_package,
        })
    return {
        "scope": "All registry packages in the locked prototype resolution, including non-macOS "
                 "dependencies; identity/licence-file checks, not vulnerability or legal approval",
        "alacritty": {
            "revision": revision,
            "tree": subprocess.check_output(git + ["rev-parse", "HEAD^{tree}"], text=True).strip(),
            "worktree_clean": True, "license": "Apache-2.0", "license_files": licenses,
        },
        "lock_sha256": sha256((ROOT / "probe/Cargo.lock").read_bytes()),
        "registry_package_count": len(packages),
        "source_file_sha256_checks": sum(item["source_file_sha256_checks"] for item in packages),
        "packages": sorted(packages, key=lambda item: (item["name"], item["version"])),
    }


if __name__ == "__main__":
    print(json.dumps(inventory(), indent=2))
