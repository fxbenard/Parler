#!/usr/bin/env zsh
# Bumpe la version, commit, tag et déclenche le workflow release CI
# Usage: ./release.sh patch|minor|major
# Exemple: ./release.sh patch  → 0.7.19 → 0.7.20

set -e
cd "$(dirname "$0")"

TYPE=${1:-patch}

# Lire la version actuelle
CURRENT=$(grep '"version"' src-tauri/tauri.conf.json | head -1 | sed 's/.*"\([0-9]*\.[0-9]*\.[0-9]*\)".*/\1/')
MAJOR=$(echo $CURRENT | cut -d. -f1)
MINOR=$(echo $CURRENT | cut -d. -f2)
PATCH=$(echo $CURRENT | cut -d. -f3)

case $TYPE in
  major) MAJOR=$((MAJOR+1)); MINOR=0; PATCH=0 ;;
  minor) MINOR=$((MINOR+1)); PATCH=0 ;;
  patch) PATCH=$((PATCH+1)) ;;
  *) echo "Usage: $0 patch|minor|major"; exit 1 ;;
esac

NEW="$MAJOR.$MINOR.$PATCH"
echo "🚀 Release: $CURRENT → $NEW"

# Bumper dans les 3 fichiers
sed -i '' "s/\"version\": \"$CURRENT\"/\"version\": \"$NEW\"/" src-tauri/tauri.conf.json package.json
sed -i '' "s/^version = \"$CURRENT\"/version = \"$NEW\"/" src-tauri/Cargo.toml

# Vérifier qu'il n'y a pas de changements non commités (hors version)
if [[ -n $(git diff --name-only | grep -v "tauri.conf.json\|package.json\|Cargo.toml") ]]; then
  echo "⚠️  Des changements non commités existent. Commit-les d'abord."
  git status --short
  exit 1
fi

# Commit + tag + push
git add src-tauri/tauri.conf.json package.json src-tauri/Cargo.toml
git commit -m "release: bump version to $NEW"
git tag "v$NEW"
git push
git push origin "v$NEW"

# Déclencher le workflow CI
gh workflow run release.yml --repo fxbenard/Parler
echo "✅ Workflow lancé. Build en cours (~30-45 min)."
echo "   Suivi : gh run list --repo fxbenard/Parler --workflow release.yml"
