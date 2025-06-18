#!/bin/bash

AX_ROOT=.arceos
COMMIT=a634925
test ! -d "$AX_ROOT" && echo "Cloning repositories ..." || true
test ! -d "$AX_ROOT" && git clone https://github.com/oscomp/arceos "$AX_ROOT" || true

git -C "$AX_ROOT" reset --hard "$COMMIT" || {
    echo "Failed to reset to commit $COMMIT. Please check the repository."
    exit 1
}

$(dirname $0)/set_ax_root.sh $AX_ROOT
