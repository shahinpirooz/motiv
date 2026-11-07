#!/usr/bin/env bash
# Set Motiv's version everywhere it appears, then commit and tag it.
#   scripts/fork/bump-version.sh 0.7.0
# Updates frontend/package.json, frontend/src-tauri/tauri.conf.json,
# frontend/src-tauri/Cargo.toml and the motiv entry in Cargo.lock, checks
# that CHANGELOG.md has a section for the version, commits "Motiv vX.Y.Z"
# and creates the annotated tag vX.Y.Z. Push with: git push origin main --tags
set -euo pipefail
VERSION="${1:-}"
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Usage: $0 X.Y.Z"; exit 1
fi
cd "$(git rev-parse --show-toplevel)"
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "Commit or stash your changes first."; exit 1
fi
if git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null; then
  echo "Tag v$VERSION already exists."; exit 1
fi
if ! grep -q "^## $VERSION" CHANGELOG.md; then
  echo "Add a '## $VERSION' section to CHANGELOG.md first."; exit 1
fi

python3 - "$VERSION" <<'EOF'
import json, re, sys
v = sys.argv[1]
def sub(path, pattern, repl, count=1):
    s = open(path).read()
    n = re.subn(pattern, repl, s, count=count, flags=re.M)
    if n[1] != 1:
        sys.exit(f"Could not update the version in {path}")
    open(path, "w").write(n[0])
sub("frontend/package.json", r'^(\s*"version":\s*")[^"]+(")', rf'\g<1>{v}\g<2>')
sub("frontend/src-tauri/tauri.conf.json", r'^(\s*"version":\s*")[^"]+(")', rf'\g<1>{v}\g<2>')
sub("frontend/src-tauri/Cargo.toml", r'^(version = ")[^"]+(")', rf'\g<1>{v}\g<2>')
sub("Cargo.lock", r'(name = "motiv"\nversion = ")[^"]+(")', rf'\g<1>{v}\g<2>')
EOF

git add frontend/package.json frontend/src-tauri/tauri.conf.json frontend/src-tauri/Cargo.toml Cargo.lock CHANGELOG.md
git commit -q -m "Motiv v$VERSION"
git tag -a "v$VERSION" -m "Motiv v$VERSION"
echo "Version $VERSION committed and tagged. Push with: git push origin main --tags"
