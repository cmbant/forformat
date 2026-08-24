#!/usr/bin/env bash
# Clone the repositories used by the corpus checks.
#
# Add or remove entries in REPOSITORIES as the corpus set changes.  The
# destination can be overridden without editing this file:
#
#   CORPORA_DIR=/somewhere/corpora ./corpora/clone_repositories.sh

set -euo pipefail

CORPORA_DIR=${CORPORA_DIR:-/tmp/corpora}
CLONE_SUBMODULES=${CLONE_SUBMODULES:-1}

# Keep the checkout directory name separate from the URL so it remains stable
# if a repository's GitHub organization or spelling changes.
REPOSITORIES=(
    "camb|https://github.com/cmbant/camb.git"
    "CosmoMC|https://github.com/cmbant/CosmoMC.git"
    "CP2K|https://github.com/cp2k/cp2k.git"
    "q-e|https://github.com/QEF/q-e.git"
    "specfem3d|https://github.com/SPECFEM/specfem3d.git"
    "OpenFAST|https://github.com/OpenFAST/openfast.git"
    "MPAS-Model|https://github.com/MPAS-Dev/MPAS-Model.git"
    "abinit|https://github.com/abinit/abinit.git"
    "WRF|https://github.com/wrf-model/WRF.git"
)

mkdir -p "$CORPORA_DIR"

for repository in "${REPOSITORIES[@]}"; do
    IFS='|' read -r name url <<<"$repository"
    destination="$CORPORA_DIR/$name"

    if [[ -e "$destination" ]]; then
        if [[ ! -d "$destination/.git" ]]; then
            printf 'error: destination exists but is not a Git checkout: %s\n' "$destination" >&2
            exit 1
        fi
        printf 'already exists, skipping: %s\n' "$destination"
        continue
    fi

    printf 'cloning %s into %s\n' "$url" "$destination"
    if [[ "$CLONE_SUBMODULES" == 1 ]]; then
        git clone --recurse-submodules "$url" "$destination"
    else
        git clone "$url" "$destination"
    fi
done
