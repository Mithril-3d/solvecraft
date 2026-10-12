#!/usr/bin/env bash
# All Pages deployments include the book, web app, and published Flatpak repository.
# Shared by docs updates and release publication; no other workflow replaces the site.
set -euo pipefail

site="${1:?site directory required}"
tag="${RELEASE_TAG:-}"
app_id=ai.storyteller.solvecraft
if [ -z "$tag" ]; then
  tag="$(gh api "repos/$GITHUB_REPOSITORY/releases?per_page=100" \
    --jq '[.[] | select(.draft == false and .prerelease == false)][0].tag_name // empty')"
fi
# Before the first publication the docs and browser app can still deploy.
if [ -z "$tag" ]; then exit 0; fi
if ! printf '%s' "$tag" | grep -Eq '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-((0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'; then
  echo "invalid release tag: $tag" >&2
  exit 1
fi
published="$(gh api "repos/$GITHUB_REPOSITORY/releases/tags/$tag" \
  --jq 'select(.draft == false and .published_at != null) | .tag_name')"
test "$published" = "$tag"
test -s "$site/index.html"
test -s "$site/app/index.html"

version="${tag#v}"
bundles="$(mktemp -d)"
gh release download "$tag" --repo "$GITHUB_REPOSITORY" --pattern SHA256SUMS.txt --dir "$bundles"
repo_dir="$site/flatpak/repo"
mkdir -p "$repo_dir"
ostree --repo="$repo_dir" init --mode=archive-z2
base_url="${PAGES_BASE_URL%/}"
previous_url="$base_url/flatpak/repo"
if curl --fail --silent --location --output /dev/null "$previous_url/summary"; then
  ostree --repo="$repo_dir" remote add --no-gpg-verify previous "$previous_url"
  ostree --repo="$repo_dir" pull --mirror previous
  ostree --repo="$repo_dir" remote delete previous
fi
for arch in x86_64 aarch64; do
  asset="solvecraft-$version-linux-$arch.flatpak"
  gh release download "$tag" --repo "$GITHUB_REPOSITORY" --pattern "$asset" --dir "$bundles"
  (
    cd "$bundles"
    awk -v file="$asset" '$2 == file || $2 == "*" file { print; found=1 } END { exit !found }' \
      SHA256SUMS.txt | sha256sum --check -
  )
  validation_repo="$(mktemp -d)"
  ostree --repo="$validation_repo" init --mode=archive-z2
  flatpak build-import-bundle "$validation_repo" "$bundles/$asset"
  ostree --repo="$validation_repo" refs | grep -Fxq "app/$app_id/$arch/stable"
  flatpak build-import-bundle "$repo_dir" "$bundles/$asset"
done
flatpak build-update-repo --title="SolidCraft" --comment='Official SolidCraft releases' \
  --description='Official Flatpak repository for SolidCraft' \
  --homepage="https://github.com/$GITHUB_REPOSITORY" --default-branch=stable "$repo_dir"
test -s "$repo_dir/summary"
cat >"$site/flatpak/solvecraft.flatpakrepo" <<META
[Flatpak Repo]
Title=SolidCraft
Comment=Official SolidCraft releases
Description=Official Flatpak repository for SolidCraft
Url=$base_url/flatpak/repo/
Homepage=https://github.com/$GITHUB_REPOSITORY
META
cat >"$site/flatpak/index.html" <<HTML
<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>SolidCraft Flatpak repository</title></head><body><main><h1>SolidCraft Flatpak repository</h1><p>Latest published release: <strong>$tag</strong></p><p><a href="solvecraft.flatpakrepo">Add the SolidCraft repository</a></p><p>Flathub provides the required Freedesktop runtime. Add both remotes to the same user installation:</p><pre>flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak remote-add --user --if-not-exists solvecraft $base_url/flatpak/solvecraft.flatpakrepo
flatpak install --user solvecraft $app_id
flatpak run $app_id</pre></main></body></html>
HTML
touch "$site/.nojekyll"
