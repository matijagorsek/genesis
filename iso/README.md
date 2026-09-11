# ISO and disk images

Produced from the OCI image with bootc-image-builder. Needs a Linux host with loop
devices; on the Mac this runs inside Docker Desktop's VM, x86_64 emulated, and is slow.
GitHub Actions (x86_64 runner) is the authoritative path in Phase 1.

    just image      # build localhost/genesis:0.1
    just iso        # anaconda-iso into iso/output/
    just qcow2      # qcow2 disk image for QEMU/UTM boot tests
