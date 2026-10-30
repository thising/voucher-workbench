#!/bin/sh
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
if [ -x "$project_root/.local/cargo/bin/cargo" ]; then
  export CARGO_HOME="$project_root/.local/cargo"
  export RUSTUP_HOME="$project_root/.local/rustup"
  export PATH="$CARGO_HOME/bin:$PATH"
fi
cd "$project_root/app"
exec cargo "$@"
