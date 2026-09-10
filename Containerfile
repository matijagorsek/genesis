# Genesis OS image — Phase 1 skeleton. NOT YET BUILT OR TESTED.
# Base: Aurora (KDE Plasma on Fedora bootc) from Universal Blue.
# Build:  podman build --platform linux/amd64 -t ghcr.io/<you>/genesis:44 .
# ISO:    osbuild/bootc-image-builder → anaconda-iso (see .github/workflows in Phase 1)

ARG BASE=ghcr.io/ublue-os/aurora-main:stable
FROM ${BASE}

# system files: units, sysusers, tmpfiles, /etc/genesis defaults
COPY system_files/ /

# inference stack (Vulkan build of llama.cpp from Fedora repos; CUDA/ROCm via ramalama containers)
RUN dnf install -y llama-cpp bubblewrap ramalama distrobox && dnf clean all

# llama-swap release binary
ARG LLAMA_SWAP_VERSION=255
RUN curl -fsSL "https://github.com/mostlygeek/llama-swap/releases/download/v${LLAMA_SWAP_VERSION}/llama-swap_${LLAMA_SWAP_VERSION}_linux_amd64.tar.gz" \
    | tar -xz -C /usr/bin llama-swap && chmod 0755 /usr/bin/llama-swap

RUN systemctl enable genesis-router.socket \
 && echo 'ID=genesis' > /usr/lib/os-release.genesis   # TODO(phase1): proper os-release rebrand, ID_LIKE=fedora

RUN bootc container lint
