#!/usr/bin/env bash
# Installs a GitHub Actions self-hosted runner on this Mac for ptescher/sim001, as a launchd
# service that starts at login. Run it yourself: it registers your machine with the repository.
#
#   tools/runner/setup.sh            # install and start
#   tools/runner/setup.sh remove     # stop, uninstall and unregister
#
# Needs `gh` logged in with admin rights on the repository (for the registration token).
set -euo pipefail

REPO="${REPO:-ptescher/sim001}"
DIR="${RUNNER_DIR:-$HOME/actions-runner-sim001}"
VERSION="${RUNNER_VERSION:-}"   # empty: the latest release

if [ "${1:-}" = remove ]; then
  cd "$DIR"
  ./svc.sh stop || true
  ./svc.sh uninstall || true
  ./config.sh remove --token "$(gh api -X POST "repos/$REPO/actions/runners/remove-token" --jq .token)"
  exit 0
fi

[ "$(uname -m)" = arm64 ] || { echo "this script is for Apple silicon"; exit 1; }
[ -n "$VERSION" ] || VERSION="$(gh api repos/actions/runner/releases/latest --jq .tag_name | sed 's/^v//')"

mkdir -p "$DIR" && cd "$DIR"
if [ ! -x ./config.sh ]; then
  curl -fsSL -o runner.tar.gz \
    "https://github.com/actions/runner/releases/download/v${VERSION}/actions-runner-osx-arm64-${VERSION}.tar.gz"
  tar xzf runner.tar.gz && rm runner.tar.gz
fi

./config.sh --unattended --replace \
  --url "https://github.com/$REPO" \
  --token "$(gh api -X POST "repos/$REPO/actions/runners/registration-token" --jq .token)" \
  --name "$(hostname -s)-sim001" \
  --labels sim001 \
  --work _work

# A launchd service runs without the shell's PATH: give it cargo and Homebrew.
echo "PATH=$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin" > .path
cp .path .env
./svc.sh install
./svc.sh start
./svc.sh status
