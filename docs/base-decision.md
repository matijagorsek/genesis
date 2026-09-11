# Own distro from scratch vs. derived from Fedora

Due diligence, 2026-09-11. Question from the founder: "be 150% sure, own ISO or Fedora?"

## Answer

**Derive from Fedora bootc via Universal Blue tooling, as a swappable base, not a marriage.**
Confidence about 90% against from-scratch or independent for a 1 to 3 person team targeting
consumer NVIDIA/AMD/Intel PCs with Secure Boot. Confidence about 75% that Fedora bootc beats
the runner-up, Ubuntu 26.04 LTS. Nothing in the record supports from-scratch at this team size.

Genesis is still our own distribution: own name, image, installer, services, desktop
configuration, kernel if we want, repositories. Only the word "Fedora" and its logo are
off limits, per Fedora's Remix rules.

## The option spectrum

| | (a) From scratch | (b) Independent, reuse upstream sources | (c) Classic derived | (d) Image-based derived |
|---|---|---|---|---|
| Examples | LFS, Yocto, Chimera, AerynOS | Alpine, Void, Solus, NixOS, Clear Linux | Ubuntu, Pop!_OS, Mint, CachyOS, Omarchy, SteamOS | Bazzite, Bluefin, Aurora, Ubuntu Core, ChromeOS |
| You own | Toolchain, package format, every build, kernel, security tracking | Own package builds, own kernel, own repo | Upstream binaries plus your overlay, installer, defaults | Upstream binaries composed in a Containerfile; your layer on top |
| Time to usable desktop with GPU drivers and Secure Boot | 3 to 5 years | 2 to 4 years | 2 to 6 months | Weeks |
| Ongoing FTE to stay current and secure | 3 to 6 | 2 to 4 | 0.5 to 1.5 | 0.3 to 1 |

## Evidence that decided it

**Independent distros take years and stay fragile.**
- Chimera: one owner, started 2021, alpha 2023, beta Dec 2024, no 1.0 in 2026.
- Serpent OS / AerynOS: started 2020, still alpha in 2026; founder absent six months in 2025.
- Solus: three-month total outage in 2023 because infrastructure had a single maintainer.
- Void: lead developer vanished in 2018 holding the domain and GitHub org.
- Clear Linux: Intel-funded for ten years, killed overnight in July 2025.

**Derived, image-based distros ship with tiny teams.**
- Bluefin and Aurora: 3 maintainers at 50k weekly active users.
- Omarchy: 2 listed developers, script in June 2025 to full ISO plus repo within months.
- CachyOS: started with 2 to 3 people.

**Security volume is not survivable alone.**
- 48,185 CVEs in 2025; the Linux kernel alone issued 5,708.
- Debian shipped 258 security advisories in 2025 with a 10-person security team.
- Alpine's security process was one volunteer until corporate funding in 2021.
- A derived image rebuilds nightly and inherits every upstream fix with zero action from us.
  Our residual burden is only the packages and daemons we add.

**Hardware enablement.** Kernel, firmware, Mesa, ROCm and the Microsoft-signed shim all come
from Fedora. NVIDIA modules come prebuilt and signed by Universal Blue's akmods with a
one-time MOK enrolment. An own shim means a legal entity, key custody, and a months-long
review. Ubuntu is the only base with NVIDIA modules signed under the distro's own
Microsoft-chained key.

**AI-native precedent.** Omarchy chose Arch. Bluefin GDX chose Fedora then CentOS Stream.
Canonical is adding local-inference features to Ubuntu itself. No AI-native distro was built
from scratch or on an independent base. MAGI OS, a one-developer Debian-based attempt, is
already abandoned.

## Base comparison for the derived route

| Base | Drivers | Image tooling | Support | AI/ML packaging | Verdict |
|---|---|---|---|---|---|
| Fedora bootc / Universal Blue | Freshest Mesa; NVIDIA via signed akmods | Best: Containerfile, GHCR, bootc, turnkey GitHub Actions | 13 months per release, rebase twice a year | ROCm and llama.cpp in-distro; CUDA via NVIDIA repo | **Chosen** |
| CentOS Stream 10 (Bluefin LTS pattern) | Older kernel | Same tooling | To 2030 | EPEL | Second "stable" stream later; no Secure Boot images yet |
| Ubuntu 26.04 LTS | NVIDIA signed out of the box; CUDA and ROCm in the archive | No shipping immutable desktop; snaps | 5 years | Strongest | Runner-up; loses on image tooling, and Canonical is building its own AI layer on it |
| Arch | Freshest | None built in | Rolling | AUR, unreviewed | Fine for devs, worst for a consumer PC that must not break |
| NixOS | Fine; CUDA not cached upstream | Declarative | 6 months | Costly to cache | Strong future option for a system an agent reconfigures; too steep for v1 |
| openSUSE MicroOS | Proprietary NVIDIA plus Secure Boot is a known pain | Good | Rolling | Modest | Disqualified on NVIDIA |

## Known risks of the chosen route and mitigations

- Fedora's 13-month cycle forces a rebase every 6 to 12 months. Mitigation: CentOS Stream LTS stream later.
- Universal Blue is a community project with 3 owners and no visible legal entity. Mitigation: everything is public Containerfiles, and Fedora's own bootc base images arrive with the Image Mode Phase 2 initiative (F44 beta, F45 production). We can rebase onto those directly.
- bootc churn: expect a few breaking rebases per year.
- The agent itself is the largest attack surface. Omarchy shipped a 14-month root escalation through a default docker group. Our security hours go to the permission model, not to redoing glibc updates.

## Exit path, so the base stays a `FROM` line

1. Own kernel builds inside the image (Bazzite, CachyOS do this). Weeks.
2. Own package overlay repo (COPR, then self-hosted). Weeks.
3. Own base image from Fedora's official bootc base, bypassing Universal Blue. Days.
4. Swap base entirely (Bluefin cloned onto CentOS Stream with the same 3 people). Months.
5. Full independence (Chimera scale). Years.

Discipline that keeps this cheap: everything Genesis-specific lives in our Containerfile
layer, our own repo, and our own daemons. Never fork Fedora packages.

## When from-scratch would be right

None apply today.
- Ten or more funded engineers and a five-year runway, and even then it can be killed overnight.
- The product cannot be expressed as packages plus config on a mainstream base, for example a non-glibc or non-systemd design that is itself the product.
- Custom hardware you sell, where Yocto is the right tool.
- Upstream policy blocks the core feature. Fedora ships ROCm and llama.cpp today, so no conflict exists.

## Unverified

Exact headcounts for Alpine, Void and CachyOS; whether any Universal Blue maintainer is paid;
"CUDA and ROCm in Ubuntu 26.04 main" is from secondary coverage.

## Sources

Chimera news · fossforce.com on AerynOS · getsol.us "A New Voyage" · itsfoss.com on Void ·
lwn.net/Articles/1030563 (Clear Linux) · dosu.dev Bluefin case study · docs.projectbluefin.io
four-years post · en.wikipedia.org/wiki/Omarchy · 0xcc.io Omarchy root escalation ·
jerrygamblin.com 2025 CVE review · tuxcare.com kernel CVE flood · lists.debian.org DSAs 2025 ·
ariadne.space security response team · github.com/rhboot/shim-review ·
universal-blue.discourse.group MOK management · fedoraproject.org/wiki/Remix ·
fedoraproject.org Image Mode Phase 2 · endoflife.date/centos-stream ·
packages.fedoraproject.org rocm · wiki.nixos.org/wiki/CUDA · kalpadesktop.org NVIDIA docs ·
phoronix.com Ubuntu AI features 2026 · github.com/ruapotato/MAGI · ezurio.com Yocto vs Buildroot
