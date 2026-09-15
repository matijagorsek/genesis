# Genesis OS image. Derived from Universal Blue Aurora (KDE Plasma on Fedora bootc).
#
#   build:  docker buildx build --platform linux/amd64 -t genesis:0.1 .
#   check:  docker run --rm --platform linux/amd64 genesis:0.1 genesis-image-check
#   iso:    see iso/ (bootc-image-builder), Phase 1

ARG BASE=ghcr.io/ublue-os/aurora:stable@sha256:1faf35ec2a253c445e3802d946ee75d37fd9b61935398ba5084c7f488ba6db14

# ---- stage 1: Genesis daemons (Rust), cross-compiled for the target architecture on whatever the build host is
FROM --platform=$BUILDPLATFORM docker.io/library/rust:1-bookworm AS daemons
ARG TARGETARCH
RUN apt-get update -q && apt-get install -y -q --no-install-recommends gcc-x86-64-linux-gnu libc6-dev-amd64-cross gcc-aarch64-linux-gnu libc6-dev-arm64-cross >/dev/null && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
WORKDIR /src
COPY src/ /src/
RUN case "${TARGETARCH:-amd64}" in arm64) T=aarch64-unknown-linux-gnu;; *) T=x86_64-unknown-linux-gnu;; esac; \
    cargo build --release --locked --target $T -p genesis-permd -p genesis-probe -p genesis-firstrun -p genesis-agentd -p genesis-txd -p genesis-krunner -p genesis-ask -p genesis-companiond \
 && for b in genesis-permd genesis-probe genesis-firstrun genesis-agentd genesis-txd genesis-krunner genesis-ask genesis-companiond; do install -D -m 0755 target/$T/release/$b /out/usr/bin/$b; done

# ---- stage 1b: genesis-window (Qt WebEngine), built on Fedora so it links against the image's Qt ------
FROM quay.io/fedora/fedora:44 AS qtbuild
RUN dnf install -y --setopt=install_weak_deps=False cmake gcc-c++ ninja-build qt6-qtbase-devel qt6-qtwebengine-devel qt6-qtdeclarative-devel extra-cmake-modules kf6-kcmutils-devel kf6-ki18n-devel >/dev/null && dnf clean all
COPY src/genesis-window/ /src/genesis-window/
COPY src/genesis-kcm/ /src/genesis-kcm/
RUN cmake -S /src/genesis-window -B /build -G Ninja -DCMAKE_BUILD_TYPE=Release >/dev/null && cmake --build /build >/dev/null \
 && install -D -m 0755 /build/genesis-window /out/usr/bin/genesis-window
# the System Settings module: installs its plugin and QML package under /out (KDE install dirs => /usr)
RUN cmake -S /src/genesis-kcm -B /build-kcm -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr -DKDE_INSTALL_USE_QT_SYS_PATHS=ON >/dev/null \
 && cmake --build /build-kcm >/dev/null && DESTDIR=/out cmake --install /build-kcm >/dev/null \
 && find /out -name "kcm_genesis*" | head -5

# whisper.cpp: local speech-to-text for voice input (Fedora ships only the library, no CLI)
FROM quay.io/fedora/fedora:44 AS whisperbuild
ARG WHISPER_CPP_VERSION=v1.8.1
RUN dnf install -y --setopt=install_weak_deps=False cmake gcc-c++ ninja-build git >/dev/null && dnf clean all
RUN git clone --depth 1 --branch ${WHISPER_CPP_VERSION} https://github.com/ggml-org/whisper.cpp /src >/dev/null 2>&1 \
 && cmake -S /src -B /build -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_EXAMPLES=ON -DGGML_NATIVE=OFF >/dev/null \
 && cmake --build /build --target whisper-cli >/dev/null \
 && mkdir -p /out/usr/lib/genesis/whisper/bin && cp /build/bin/whisper-cli /out/usr/lib/genesis/whisper/bin/

# llama.cpp for arm64: built from source with every CPU variant (chosen at run time: dotprod, i8mm, SVE)
# and the Vulkan backend. The upstream arm64 tarball is a generic build and about 2-3x slower on Apple
# Silicon and modern ARM. amd64 keeps the upstream Vulkan tarball (it already ships all CPU variants).
FROM quay.io/fedora/fedora:44 AS llamabuild
ARG TARGETARCH
ARG LLAMA_CPP_BUILD=b10901
RUN set -eux; mkdir -p /out; if [ "$TARGETARCH" = arm64 ]; then \
      dnf install -y --setopt=install_weak_deps=False cmake gcc-c++ ninja-build git curl libcurl-devel vulkan-headers vulkan-loader-devel glslc glslang spirv-headers-devel spirv-tools-devel; dnf clean all; \
      git clone --depth 1 --branch ${LLAMA_CPP_BUILD} https://github.com/ggml-org/llama.cpp /src; \
      cmake -S /src -B /build -G Ninja -DCMAKE_BUILD_TYPE=Release -DGGML_NATIVE=OFF -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON -DGGML_VULKAN=ON \
        -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_TOOLS=ON -DLLAMA_CURL=ON; \
      cmake --build /build -j 3 --target llama-server llama-cli llama-bench llama-quantize llama-mtmd-cli; \
      mkdir -p /out/usr/lib/genesis/llama.cpp; cp /build/bin/llama-* /build/bin/*.so* /out/usr/lib/genesis/llama.cpp/; \
      ls /out/usr/lib/genesis/llama.cpp; \
    fi

# ---- stage 2: the OS image ------------------------------------------------------------------------
# gopls for Babel: built from the pinned tag, the only Go on the image is this one binary
FROM quay.io/fedora/fedora:44 AS goplsbuild
RUN dnf install -y --setopt=install_weak_deps=False golang git >/dev/null && dnf clean all \
 && GOFLAGS=-mod=mod GOPATH=/go GOBIN=/out go install golang.org/x/tools/gopls@v0.23.0 >/dev/null 2>&1 && /out/gopls version

FROM ${BASE}

# FLAVOUR=aurora: Universal Blue Aurora already brings the Plasma desktop (x86_64 only).
# FLAVOUR=fedora: plain Fedora bootc (multi-arch, used for arm64); we install the Plasma desktop ourselves.
ARG FLAVOUR=aurora
ARG TARGETARCH
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

# ---- desktop for the plain Fedora bootc flavour (Aurora already has it) -----------------------
RUN set -eux; if [ "$FLAVOUR" = fedora ]; then \
      dnf5 install -y --setopt=install_weak_deps=False \
        plasma-desktop plasma-workspace plasma-workspace-wayland kwin plasma-login-manager kde-settings kde-settings-plasma \
        plasma-nm plasma-pa plasma-systemmonitor plasma-disks kscreen powerdevil bluedevil kdeplasma-addons plasma-browser-integration \
        xdg-desktop-portal-kde polkit-kde kwallet-pam breeze-gtk-gtk3 breeze-gtk-gtk4 kde-gtk-config \
        dolphin konsole kate ark spectacle kinfocenter \
        pipewire pipewire-pulse pipewire-alsa wireplumber NetworkManager-wifi firewalld flatpak distrobox bubblewrap \
        openssh-server plymouth-system-theme mesa-vulkan-drivers mesa-dri-drivers \
        google-noto-sans-fonts google-noto-emoji-color-fonts google-noto-sans-mono-fonts jq wl-clipboard libnotify \
        udisks2 upower fwupd bluez avahi cups system-config-printer sddm-kcm 2>&1 | tail -3; \
      dnf5 clean all; \
      systemctl set-default graphical.target; \
      systemctl enable --force plasmalogin.service; \
      systemctl enable NetworkManager firewalld; \
      firewall-offline-cmd --zone=public --add-service=kdeconnect >/dev/null 2>&1 || true; \
      firewall-offline-cmd --zone=public --add-port=11530/tcp >/dev/null 2>&1 || true; \
    fi

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
COPY --from=llamabuild /out/ /
RUN set -eux; \
    mkdir -p /usr/lib/genesis/llama.cpp; \
    if [ "${TARGETARCH:-amd64}" != arm64 ]; then \
      curl -fsSL --retry 5 --retry-all-errors --retry-delay 10 "https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_CPP_BUILD}/llama-${LLAMA_CPP_BUILD}-bin-ubuntu-vulkan-x64.tar.gz" \
        | tar -xz -C /usr/lib/genesis/llama.cpp --strip-components=1; \
    fi; \
    for b in llama-server llama-cli llama-bench llama-embedding llama-quantize llama-mtmd-cli; do \
      [ -x "/usr/lib/genesis/llama.cpp/$b" ] || continue; \
      printf '#!/bin/sh\nexport LD_LIBRARY_PATH=/usr/lib/genesis/llama.cpp${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}\nexport GGML_BACKEND_PATH=/usr/lib/genesis/llama.cpp\nexec /usr/lib/genesis/llama.cpp/%s "$@"\n' "$b" > "/usr/bin/$b"; \
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
      rsms-inter-fonts ibm-plex-mono-fonts papirus-icon-theme papirus-icon-theme-dark papirus-icon-theme-light ocean-sound-theme \
      chromium firefox okular gwenview kcalc plasma-discover plasma-discover-flatpak haruna elisa kcharselect kfind \
      kdeconnect-kde kwalletmanager5 partitionmanager rsync python3-pytest ffmpeg-free qrencode rust-analyzer clang-tools-extra lldb delve; \
    dnf5 clean all

# Piper: local text-to-speech (static upstream build with its espeak-ng data and onnxruntime)
RUN set -eux; \
    mkdir -p /usr/lib/genesis; \
    case "${TARGETARCH:-amd64}" in arm64) PA=aarch64;; *) PA=x86_64;; esac; \
    case "$PA" in aarch64) SUM=fea0fd2d87c54dbc7078d0f878289f404bd4d6eea6e7444a77835d1537ab88eb;; *) SUM=a50cb45f355b7af1f6d758c1b360717877ba0a398cc8cbe6d2a7a3a26e225992;; esac; \
    curl -fsSL --retry 5 --retry-all-errors --retry-delay 10 -o /tmp/piper.tgz "https://github.com/rhasspy/piper/releases/download/${PIPER_VERSION}/piper_linux_${PA}.tar.gz"; \
    echo "$SUM  /tmp/piper.tgz" | sha256sum -c -; tar -xzf /tmp/piper.tgz -C /usr/lib/genesis; rm -f /tmp/piper.tgz; \
    test -x /usr/lib/genesis/piper/piper

# Babel: the Genesis IDE. Code-OSS through VSCodium (MIT, no telemetry), every language VS Code speaks,
# with the Genesis extension built in: the maker in the sidebar, permission cards, ask about the selection.
ARG VSCODIUM_VERSION=1.135.06055
RUN set -eux; \
    case "${TARGETARCH:-amd64}" in arm64) VA=arm64; SUM=9765cea4f707ff7dc83a40be408a7318a59abb6996b359631639d9aab2f48a90;; *) VA=x64; SUM=c09d8ac8dd7f52b09ee159ee24b440541dfd8f937a0f6f88cc428c78e48ee1f2;; esac; \
    curl -fsSL --retry 5 --retry-all-errors --retry-delay 10 -o /tmp/babel.tgz "https://github.com/VSCodium/vscodium/releases/download/${VSCODIUM_VERSION}/VSCodium-linux-${VA}-${VSCODIUM_VERSION}.tar.gz"; \
    echo "$SUM  /tmp/babel.tgz" | sha256sum -c -; \
    mkdir -p /usr/lib/babel; tar -xzf /tmp/babel.tgz -C /usr/lib/babel; rm -f /tmp/babel.tgz; \
    /usr/bin/genesis-babel-brand; \
    /usr/bin/genesis-babel-extensions; \
    test -x /usr/bin/babel && test -x /usr/lib/babel/bin/codium
COPY --from=goplsbuild /out/gopls /usr/bin/gopls

# llama-swap: model router (Go, static upstream binary)
RUN set -eux; \
    case "${TARGETARCH:-amd64}" in arm64) SUM=98686bc626e2d3df3b340b963fd4e4f4d3dd02dcd1bf31f0c777fb09e3053288;; *) SUM=84aa0df0cf3e302a8591e39de347f64c0c7dce1c3a948df68723a82e1fb4f1d4;; esac; \
    curl -fsSL --retry 5 --retry-all-errors --retry-delay 10 -o /tmp/llama-swap.tgz "https://github.com/mostlygeek/llama-swap/releases/download/v${LLAMA_SWAP_VERSION}/llama-swap_${LLAMA_SWAP_VERSION}_linux_${TARGETARCH:-amd64}.tar.gz"; \
    echo "$SUM  /tmp/llama-swap.tgz" | sha256sum -c -; tar -xzf /tmp/llama-swap.tgz -C /usr/bin llama-swap; rm -f /tmp/llama-swap.tgz; \
    chmod 0755 /usr/bin/llama-swap; \
    /usr/bin/llama-swap --version

# (genesis-image-check ships from system_files/usr/bin; podman/buildah has no COPY heredoc)

# ---- enable services -------------------------------------------------------------------------
RUN systemctl enable genesis-router.service genesis-probe.service genesis-firstrun.service genesis-packs-refresh.service genesis-devssh.service genesis-bootc-status.service bootc-fetch-apply-updates.timer && systemctl --global enable genesis-phone.service genesis-companiond.service genesis-packs.timer genesis-index.timer genesis-permd.service genesis-agentd.service \
 && (systemctl mask plasma-setup.service || true)

# ---- bootc validation ------------------------------------------------------------------------
RUN if [ "$BOOTC_LINT" = strict ]; then bootc container lint; else echo "bootc lint skipped (BOOTC_LINT=$BOOTC_LINT)"; fi
