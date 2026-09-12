# Genesis OS image. Derived from Universal Blue Aurora (KDE Plasma on Fedora bootc).
#
#   build:  docker buildx build --platform linux/amd64 -t genesis:0.1 .
#   check:  docker run --rm --platform linux/amd64 genesis:0.1 genesis-image-check
#   iso:    see iso/ (bootc-image-builder), Phase 1

ARG BASE=ghcr.io/ublue-os/aurora:stable

# ---- stage 1: Genesis daemons (Rust), cross-compiled for x86_64 on whatever the build host is -------
FROM --platform=$BUILDPLATFORM docker.io/library/rust:1-bookworm AS daemons
RUN apt-get update -q && apt-get install -y -q --no-install-recommends gcc-x86-64-linux-gnu libc6-dev-amd64-cross >/dev/null && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-gnu
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc
WORKDIR /src
COPY src/ /src/
RUN cargo build --release --target x86_64-unknown-linux-gnu -p genesis-permd -p genesis-probe -p genesis-firstrun -p genesis-agentd -p genesis-txd -p genesis-krunner \
 && for b in genesis-permd genesis-probe genesis-firstrun genesis-agentd genesis-txd genesis-krunner; do install -D -m 0755 target/x86_64-unknown-linux-gnu/release/$b /out/usr/bin/$b; done

# ---- stage 1b: genesis-window (Qt WebEngine), built on Fedora so it links against the image's Qt ------
FROM quay.io/fedora/fedora:44 AS qtbuild
RUN dnf install -y --setopt=install_weak_deps=False cmake gcc-c++ ninja-build qt6-qtbase-devel qt6-qtwebengine-devel >/dev/null && dnf clean all
COPY src/genesis-window/ /src/genesis-window/
RUN cmake -S /src/genesis-window -B /build -G Ninja -DCMAKE_BUILD_TYPE=Release >/dev/null && cmake --build /build >/dev/null \
 && install -D -m 0755 /build/genesis-window /out/usr/bin/genesis-window

# whisper.cpp: local speech-to-text for voice input (Fedora ships only the library, no CLI)
FROM quay.io/fedora/fedora:44 AS whisperbuild
ARG WHISPER_CPP_VERSION=v1.8.1
RUN dnf install -y --setopt=install_weak_deps=False cmake gcc-c++ ninja-build git >/dev/null && dnf clean all
RUN git clone --depth 1 --branch ${WHISPER_CPP_VERSION} https://github.com/ggml-org/whisper.cpp /src >/dev/null 2>&1 \
 && cmake -S /src -B /build -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_EXAMPLES=ON -DGGML_NATIVE=OFF >/dev/null \
 && cmake --build /build --target whisper-cli >/dev/null \
 && mkdir -p /out/usr/lib/genesis/whisper/bin && cp /build/bin/whisper-cli /out/usr/lib/genesis/whisper/bin/

# ---- stage 2: the OS image ------------------------------------------------------------------------
FROM ${BASE}

ARG GENESIS_VERSION=0.1
ARG LLAMA_SWAP_VERSION=255
ARG PIPER_VERSION=2023.11.14-2
# BOOTC_LINT=strict (CI, native amd64) | skip (local emulated builds: lint needs syscalls QEMU lacks)
ARG BOOTC_LINT=strict

# ---- identity (Fedora Remix rules: own name, no Fedora marks) -----------------------------
RUN set -eux; \
    sed -i \
      -e 's/^NAME=.*/NAME="Genesis"/' \
      -e "s/^PRETTY_NAME=.*/PRETTY_NAME=\"Genesis ${GENESIS_VERSION} (Fedora bootc 44)\"/" \
      -e 's/^ID=.*/ID=genesis/' \
      -e 's/^ID_LIKE=.*/ID_LIKE="fedora"/' \
      -e 's/^VARIANT=.*/VARIANT="Genesis Desktop"/' \
      -e 's/^VARIANT_ID=.*/VARIANT_ID=genesis/' \
      -e 's|^HOME_URL=.*|HOME_URL="https://github.com/matijagorsek/genesis"|' \
      -e 's|^SUPPORT_URL=.*|SUPPORT_URL="https://github.com/matijagorsek/genesis/issues"|' \
      -e 's|^BUG_REPORT_URL=.*|BUG_REPORT_URL="https://github.com/matijagorsek/genesis/issues"|' \
      -e 's/^IMAGE_ID=.*/IMAGE_ID=genesis/' \
      -e 's/^DEFAULT_HOSTNAME=.*/DEFAULT_HOSTNAME=genesis/' \
      -e "s/^IMAGE_VERSION=.*/IMAGE_VERSION=${GENESIS_VERSION}/" \
      /usr/lib/os-release; \
    grep -q '^ID_LIKE=' /usr/lib/os-release || echo 'ID_LIKE="fedora"' >> /usr/lib/os-release; \
    grep -q '^DEFAULT_HOSTNAME=' /usr/lib/os-release || echo 'DEFAULT_HOSTNAME=genesis' >> /usr/lib/os-release; \
    grep -q '^IMAGE_ID=' /usr/lib/os-release || printf 'IMAGE_ID=genesis\nIMAGE_VERSION=%s\n' "${GENESIS_VERSION}" >> /usr/lib/os-release

# ---- Genesis files: units, sysusers, tmpfiles, policy, /etc/genesis defaults ---------------
COPY system_files/ /
COPY packs/ /usr/share/genesis/packs/
COPY templates/ /usr/share/genesis/templates/
COPY --from=daemons /out/ /
COPY --from=qtbuild /out/ /
COPY --from=whisperbuild /out/ /
# /etc/hostname ships from system_files/etc/hostname: during a container build /etc/hostname is a runtime
# bind mount, so a RUN that writes it never reaches the layer; COPY does.

# ---- inference stack -------------------------------------------------------------------------
# llama.cpp: upstream Vulkan build (CPU + Vulkan backends, runs on NVIDIA/AMD/Intel via Mesa or vendor ICDs).
# Fedora's llama-cpp package is not used: it is months behind upstream, has no Vulkan backend, and pulls
# the entire ROCm stack (+2.5 GB) into the image. CUDA/ROCm builds come via ramalama containers instead.
ARG LLAMA_CPP_BUILD=b10901
RUN set -eux; \
    mkdir -p /usr/lib/genesis/llama.cpp; \
    curl -fsSL "https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_CPP_BUILD}/llama-${LLAMA_CPP_BUILD}-bin-ubuntu-vulkan-x64.tar.gz" \
      | tar -xz -C /usr/lib/genesis/llama.cpp --strip-components=1; \
    for b in llama-server llama-cli llama-bench llama-embedding llama-quantize llama-mtmd-cli; do \
      [ -x "/usr/lib/genesis/llama.cpp/$b" ] || continue; \
      printf '#!/bin/sh\nexport LD_LIBRARY_PATH=/usr/lib/genesis/llama.cpp${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}\nexec /usr/lib/genesis/llama.cpp/%s "$@"\n' "$b" > "/usr/bin/$b"; \
      chmod 0755 "/usr/bin/$b"; \
    done; \
    echo "${LLAMA_CPP_BUILD}" > /usr/lib/genesis/llama.cpp/BUILD; \
    /usr/bin/llama-server --version 2>&1 | head -2

# ramalama: model pulls (OCI/HF/Ollama) and containerised CUDA/ROCm runners. vulkan-tools for genesis-probe.
RUN set -eux; \
    dnf5 install -y --setopt=install_weak_deps=False ramalama vulkan-tools; \
    dnf5 remove -y plasma-welcome >/dev/null 2>&1 || true; \
    dnf5 clean all

# ---- a complete desktop out of the box: browser, documents, images, media, calculator, app store -----
# These are RPMs so they work at first boot with no network. Larger suites (office) come as Flatpaks
# from the preinstall list below once the machine is online.
RUN set -eux; \
    dnf5 install -y --setopt=install_weak_deps=False \
      chromium firefox okular gwenview kcalc plasma-discover plasma-discover-flatpak haruna elisa kcharselect kfind \
      kdeconnect-kde kwalletmanager5 partitionmanager; \
    dnf5 clean all

# Piper: local text-to-speech (static upstream build with its espeak-ng data and onnxruntime)
RUN set -eux; \
    mkdir -p /usr/lib/genesis; \
    curl -fsSL "https://github.com/rhasspy/piper/releases/download/${PIPER_VERSION}/piper_linux_x86_64.tar.gz" | tar -xz -C /usr/lib/genesis; \
    test -x /usr/lib/genesis/piper/piper

# llama-swap: model router (Go, static upstream binary)
RUN set -eux; \
    curl -fsSL "https://github.com/mostlygeek/llama-swap/releases/download/v${LLAMA_SWAP_VERSION}/llama-swap_${LLAMA_SWAP_VERSION}_linux_amd64.tar.gz" \
      | tar -xz -C /usr/bin llama-swap; \
    chmod 0755 /usr/bin/llama-swap; \
    /usr/bin/llama-swap --version

# (genesis-image-check ships from system_files/usr/bin; podman/buildah has no COPY heredoc)

# ---- enable services -------------------------------------------------------------------------
RUN systemctl enable genesis-router.socket genesis-probe.service genesis-firstrun.service && systemctl --global enable genesis-permd.service genesis-agentd.service \
 && systemctl mask plasma-setup.service

# ---- bootc validation ------------------------------------------------------------------------
RUN if [ "$BOOTC_LINT" = strict ]; then bootc container lint; else echo "bootc lint skipped (BOOTC_LINT=$BOOTC_LINT)"; fi
