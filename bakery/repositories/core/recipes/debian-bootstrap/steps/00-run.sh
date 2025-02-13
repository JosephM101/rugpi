#!/bin/bash

set -euo pipefail

case "${RUGIX_ARCH}" in
    "amd64")
        DEBIAN_ARCH="amd64"
        ;;
    "arm64")
        DEBIAN_ARCH="arm64"
        ;;
    "armv7")
        DEBIAN_ARCH="armhf"
        ;;
    "arm")
        DEBIAN_ARCH="armel"
        ;;
    *)
        echo "Unsupported architecture '${RUGIX_ARCH}'."
        exit 1
esac

MIRROR=""
if [ "${RECIPE_PARAM_SNAPSHOT}" != "" ]; then 
    MIRROR="https://snapshot.debian.org/archive/debian/${RECIPE_PARAM_SNAPSHOT}/"
fi

mmdebstrap \
    --skip=check/qemu \
    --architectures="${DEBIAN_ARCH}" \
    "${RECIPE_PARAM_SUITE}" \
    "${RUGIX_ROOT_DIR}" \
    ${MIRROR}
